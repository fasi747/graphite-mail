//! Contacts for Gmail (app password) accounts, built on this computer: **People you've emailed** from the From / To /
//! Cc headers of synced mail, and **Imported** from vCard (.vcf) or Google Contacts CSV files. Nothing here talks to a
//! server; Google's own contacts need Google sign-in and come in a later update.
//!
//! Both kinds live in the `contacts` table (sources "emailed" and "imported") under the IMAP account, so the Contacts
//! view, favourites, the right-click menu, To/Cc/Bcc suggestions, "Remove cached contacts" and account removal all work
//! the same way as for Microsoft 365 contacts.

use crate::storage;
use anyhow::{Result, anyhow, bail};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::HashMap;

pub const EMAILED: &str = "emailed";
pub const IMPORTED: &str = "imported";
/// account_state key: "<max message rowid seen>:<messages up to it>" — the incremental scan's bookmark.
/// (Starts with `contacts:` so "Remove cached contacts" clears it together with the Microsoft bookmarks.)
const SCAN_KEY: &str = "contacts:people:scan";

// ---------------------------------------------------------------- who counts as a person

/// Senders that are clearly machines: no-reply boxes, bounce handlers, notification and newsletter senders,
/// mailing-list plumbing, VERP-style addresses with long tracking tokens.
pub fn is_automated(address: &str, name: Option<&str>) -> bool {
    let a = address.trim().to_lowercase();
    let Some((local, domain)) = a.split_once('@') else { return true };
    if local.is_empty() || domain.is_empty() || !domain.contains('.') {
        return true;
    }
    let base = local.split('+').next().unwrap_or(local);
    const EXACT: [&str; 40] = [
        "noreply", "no-reply", "no_reply", "no.reply", "donotreply", "do-not-reply", "do_not_reply", "do.not.reply", "dont-reply", "nereply",
        "mailer-daemon", "mailer_daemon", "postmaster", "bounce", "bounces", "bounced", "notification", "notifications", "notify", "alert",
        "alerts", "newsletter", "newsletters", "digest", "digests", "automated", "automation", "autoreply", "auto-reply", "auto_reply",
        "robot", "bot", "daemon", "unsubscribe", "nobody", "system", "noreply-dmarc-support", "drive-shares-noreply", "comments-noreply", "calendar-notification",
    ];
    if EXACT.contains(&base) {
        return true;
    }
    const CONTAINS: [&str; 12] = ["noreply", "no-reply", "no_reply", "donotreply", "do-not-reply", "do_not_reply", "mailer-daemon", "-bounces", "bounce-", "bounces+", "unsubscribe", "notifications"];
    if CONTAINS.iter().any(|p| base.contains(p)) {
        return true;
    }
    if base.ends_with("-request") || base.ends_with("-owner") || base.ends_with("-admin") || base.starts_with("owner-") || base.ends_with("-noreply") {
        return true;
    }
    // Tracking / VERP addresses: long local parts made mostly of digits and hex, like "bounce.12ab34cd56ef78@..."
    let hexish = base.chars().filter(|c| c.is_ascii_hexdigit()).count();
    if base.len() >= 24 && hexish * 10 >= base.len() * 7 && base.chars().filter(|c| c.is_ascii_digit()).count() >= 6 {
        return true;
    }
    if let Some(n) = name {
        let n = n.to_lowercase();
        if ["no-reply", "noreply", "do not reply", "donotreply", "mailer-daemon", "mail delivery", "automated"].iter().any(|p| n.contains(p)) {
            return true;
        }
    }
    false
}

/// One person named in a message header.
#[derive(Debug, Clone, PartialEq)]
pub struct Person {
    pub name: Option<String>,
    pub address: String,
}

fn clean_name(n: Option<&str>) -> Option<String> {
    let n = n?.trim().trim_matches('"').trim();
    (!n.is_empty() && !n.contains('@')).then(|| n.to_string())
}

fn valid_address(a: &str) -> bool {
    let a = a.trim();
    a.len() >= 3 && a.len() <= 254 && a.matches('@').count() == 1 && !a.starts_with('@') && !a.ends_with('@') && !a.chars().any(|c| c.is_whitespace() || c == '<' || c == '>' || c == ',' || c == ';')
}

/// Everyone a stored message names: its sender, then To and Cc (the JSON lists the app stores: `[{name, address}]`).
pub fn people_in(sender_name: Option<&str>, sender_address: Option<&str>, to_json: Option<&str>, cc_json: Option<&str>) -> Vec<Person> {
    let mut out = vec![];
    if let Some(a) = sender_address.filter(|a| valid_address(a)) {
        out.push(Person { name: clean_name(sender_name), address: a.trim().to_string() });
    }
    for json in [to_json, cc_json].into_iter().flatten() {
        let list: Vec<Value> = serde_json::from_str(json).unwrap_or_default();
        for p in list {
            if let Some(a) = p["address"].as_str().filter(|a| valid_address(a)) {
                out.push(Person { name: clean_name(p["name"].as_str()), address: a.trim().to_string() });
            }
        }
    }
    out
}

// ---------------------------------------------------------------- "People you've emailed"

struct Agg {
    name: Option<String>,
    name_at: i64,
    count: i64,
    last: i64,
}

fn read_scan(db: &rusqlite::Connection, account_id: &str) -> Option<(i64, i64)> {
    let v: Option<String> = db.query_row("SELECT value FROM account_state WHERE account_id=?1 AND key=?2", params![account_id, SCAN_KEY], |r| r.get(0)).optional().ok()?;
    let v = v?;
    let (a, b) = v.split_once(':')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

/// Builds or extends the "People you've emailed" list of one IMAP account from the messages synced since the last
/// run (a full rebuild when older messages were deleted, so counts never drift). Returns how many people changed.
/// Skipped while the user has removed the cached contacts (until "Refresh contacts").
pub fn rebuild_people(account_id: &str) -> Result<usize> {
    storage::with_db(|db| {
        let paused = db.query_row("SELECT 1 FROM account_state WHERE account_id=?1 AND key='contacts:paused'", [account_id], |_| Ok(())).optional()?.is_some();
        if paused {
            return Ok(0);
        }
        let own: String = db.query_row("SELECT lower(email) FROM accounts WHERE id=?1", [account_id], |r| r.get(0))?;
        let (bookmark, stored) = read_scan(db, account_id).unwrap_or((0, 0));
        let up_to_bookmark: i64 = db.query_row("SELECT COUNT(*) FROM messages WHERE account_id=?1 AND rowid<=?2", params![account_id, bookmark], |r| r.get(0))?;
        let full = up_to_bookmark != stored;
        let tx = db.unchecked_transaction()?;
        if full {
            tx.execute("DELETE FROM contacts WHERE account_id=?1 AND source=?2", params![account_id, EMAILED])?;
        }
        let since = if full { 0 } else { bookmark };
        let mut agg: HashMap<String, Agg> = HashMap::new();
        let mut max_rowid = bookmark;
        {
            let mut st = tx.prepare(
                "SELECT m.rowid, m.sender_name, m.sender_address, m.to_recipients_json, m.cc_recipients_json, m.received_at
                 FROM messages m JOIN folders f ON f.id=m.folder_id
                 WHERE m.account_id=?1 AND m.rowid>?2 AND m.is_draft=0
                   AND COALESCE(f.well_known_name,'') NOT IN ('junkemail','deleteditems','drafts')
                 ORDER BY m.rowid",
            )?;
            let rows = st.query_map(params![account_id, since], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, Option<String>>(3)?, r.get::<_, Option<String>>(4)?, r.get::<_, i64>(5)?))
            })?;
            for row in rows {
                let (rowid, sn, sa, to, cc, at) = row?;
                max_rowid = max_rowid.max(rowid);
                let mut seen_in_this_message = std::collections::HashSet::new();
                for p in people_in(sn.as_deref(), sa.as_deref(), to.as_deref(), cc.as_deref()) {
                    let key = p.address.to_lowercase();
                    if key == own || is_automated(&key, p.name.as_deref()) || !seen_in_this_message.insert(key.clone()) {
                        continue;
                    }
                    let e = agg.entry(key).or_insert(Agg { name: None, name_at: -1, count: 0, last: 0 });
                    e.count += 1;
                    e.last = e.last.max(at);
                    if p.name.is_some() && at >= e.name_at {
                        e.name = p.name;
                        e.name_at = at;
                    }
                }
            }
        }
        // The newest message decides the bookmark; rows up to it are counted so deletions are noticed next time.
        max_rowid = max_rowid.max(tx.query_row("SELECT COALESCE(MAX(rowid),0) FROM messages WHERE account_id=?1", [account_id], |r| r.get(0))?);
        let changed = agg.len();
        for (address, a) in agg {
            let name = a.name.clone().unwrap_or_else(|| address.clone());
            tx.execute(
                "INSERT INTO contacts(id,account_id,source,remote_id,display_name,emails_json,phones_json,search_text,sync_gen,message_count,last_contacted)
                 VALUES(?1,?2,?3,?4,?5,?6,'[]',?7,0,?8,?9)
                 ON CONFLICT(account_id,source,remote_id) DO UPDATE SET
                   message_count = contacts.message_count + excluded.message_count,
                   last_contacted = MAX(COALESCE(contacts.last_contacted,0), excluded.last_contacted),
                   display_name = CASE WHEN excluded.display_name <> excluded.remote_id
                                        AND (contacts.display_name = contacts.remote_id OR excluded.last_contacted >= COALESCE(contacts.last_contacted,0))
                                       THEN excluded.display_name ELSE contacts.display_name END",
                params![uuid::Uuid::new_v4().to_string(), account_id, EMAILED, address, name, json!([address]).to_string(), "", a.count, a.last],
            )?;
            tx.execute(
                "UPDATE contacts SET search_text = lower(display_name || ' ' || remote_id) WHERE account_id=?1 AND source=?2 AND remote_id=?3",
                params![account_id, EMAILED, address],
            )?;
        }
        let counted: i64 = tx.query_row("SELECT COUNT(*) FROM messages WHERE account_id=?1 AND rowid<=?2", params![account_id, max_rowid], |r| r.get(0))?;
        tx.execute(
            "INSERT INTO account_state(account_id,key,value) VALUES(?1,?2,?3) ON CONFLICT(account_id,key) DO UPDATE SET value=excluded.value",
            params![account_id, SCAN_KEY, format!("{max_rowid}:{counted}")],
        )?;
        tx.commit()?;
        Ok(changed)
    })
}

/// "Refresh contacts" for an IMAP account: ends a "Remove cached contacts" pause and rebuilds the people list.
pub fn refresh_people(account_id: &str) -> Result<usize> {
    storage::with_db(|db| Ok(db.execute("DELETE FROM account_state WHERE account_id=?1 AND key='contacts:paused'", [account_id]).map(|_| ())?))?;
    rebuild_people(account_id)
}

// ---------------------------------------------------------------- reading files: text, vCard, CSV

/// Text from a file that may be UTF-8 (with or without BOM), UTF-16 (BOM), or an old Windows / Latin-1 export.
pub fn decode_text(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    if bytes.len() >= 2 && (bytes[..2] == [0xFF, 0xFE] || bytes[..2] == [0xFE, 0xFF]) {
        let be = bytes[0] == 0xFE;
        let units: Vec<u16> = bytes[2..].chunks(2).filter(|c| c.len() == 2).map(|c| if be { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) }).collect();
        return String::from_utf16_lossy(&units);
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }
    bytes.iter().map(|&b| cp1252(b)).collect()
}

/// Windows-1252 (a superset of Latin-1: the 0x80–0x9F row holds € ‘ ’ “ ” – — … and friends).
fn cp1252(b: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}',
        '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ',
    ];
    match b {
        0x80..=0x9F => HIGH[(b - 0x80) as usize],
        _ => b as char,
    }
}

/// One contact read from a file.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Imported {
    pub name: String,
    pub given: Option<String>,
    pub surname: Option<String>,
    pub emails: Vec<String>,
    /// (kind, number)
    pub phones: Vec<(String, String)>,
    pub company: Option<String>,
    pub title: Option<String>,
}

fn add_email(list: &mut Vec<String>, e: &str) {
    let e = e.trim().trim_start_matches("mailto:").trim();
    if valid_address(e) && !list.iter().any(|x| x.eq_ignore_ascii_case(e)) {
        list.push(e.to_string());
    }
}

fn add_phone(list: &mut Vec<(String, String)>, kind: &str, n: &str) {
    let n = n.trim();
    if !n.is_empty() && !list.iter().any(|(_, x)| x == n) {
        list.push((kind.to_string(), n.to_string()));
    }
}

fn none_if_empty(s: String) -> Option<String> {
    let s = s.trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Fills in the display name from the name parts or the first address when the file gave none.
fn finish(mut c: Imported) -> Option<Imported> {
    if c.name.trim().is_empty() {
        c.name = [c.given.as_deref(), c.surname.as_deref()].into_iter().flatten().collect::<Vec<_>>().join(" ");
    }
    if c.name.trim().is_empty() {
        c.name = c.emails.first().cloned().unwrap_or_default();
    }
    c.name = c.name.trim().to_string();
    (!c.name.is_empty() || !c.emails.is_empty()).then_some(c)
}

/// Decodes a quoted-printable value (vCard 2.1), as UTF-8 or Windows-1252 bytes.
fn quoted_printable(v: &str, utf8: bool) -> String {
    let mut bytes = Vec::with_capacity(v.len());
    let b = v.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'=' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (b.get(i + 1).and_then(|c| (*c as char).to_digit(16)), b.get(i + 2).and_then(|c| (*c as char).to_digit(16))) {
                bytes.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        bytes.push(b[i]);
        i += 1;
    }
    if utf8 { String::from_utf8_lossy(&bytes).into_owned() } else { bytes.iter().map(|&c| cp1252(c)).collect() }
}

/// vCard text escapes (RFC 2426 / 6350): `\n`, `\,`, `\;`, `\\`.
fn vunescape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut it = v.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') | Some('N') => out.push('\n'),
                Some(x) => out.push(x),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Splits a compound value on unescaped `;` (N, ORG, ADR).
fn vsplit(v: &str) -> Vec<String> {
    let mut parts = vec![String::new()];
    let mut esc = false;
    for c in v.chars() {
        if esc {
            parts.last_mut().unwrap().push('\\');
            parts.last_mut().unwrap().push(c);
            esc = false;
        } else if c == '\\' {
            esc = true;
        } else if c == ';' {
            parts.push(String::new());
        } else {
            parts.last_mut().unwrap().push(c);
        }
    }
    parts.into_iter().map(|p| vunescape(&p).trim().to_string()).collect()
}

/// Logical lines of a vCard file: CRLF/LF/CR line ends, folded lines joined (a line starting with a space or tab
/// continues the previous one), quoted-printable soft breaks (a trailing `=`) joined too.
fn vlines(text: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    let mut qp_open = false;
    for raw in text.replace("\r\n", "\n").replace('\r', "\n").split('\n') {
        if (raw.starts_with(' ') || raw.starts_with('\t')) && !out.is_empty() {
            out.last_mut().unwrap().push_str(&raw[1..]);
            continue;
        }
        if qp_open && !out.is_empty() {
            let last = out.last_mut().unwrap();
            last.pop(); // the soft-break '='
            last.push_str(raw);
        } else {
            out.push(raw.to_string());
        }
        let l = out.last().unwrap();
        qp_open = l.ends_with('=') && l.to_ascii_uppercase().contains("QUOTED-PRINTABLE");
    }
    out.into_iter().filter(|l| !l.trim().is_empty()).collect()
}

/// Every contact in a vCard file (2.1, 3.0 and 4.0; several cards per file).
pub fn parse_vcards(text: &str) -> Vec<Imported> {
    let mut out = vec![];
    let mut cur: Option<Imported> = None;
    for line in vlines(text) {
        let Some(colon) = line.find(':') else { continue };
        let (head, value) = (&line[..colon], &line[colon + 1..]);
        let mut parts = head.split(';');
        let name = parts.next().unwrap_or("").trim();
        let name = name.rsplit('.').next().unwrap_or(name).to_ascii_uppercase(); // "item1.EMAIL" → EMAIL
        let params: Vec<String> = parts.map(|p| p.trim().to_ascii_uppercase()).collect();
        let types: Vec<String> = params
            .iter()
            .flat_map(|p| match p.split_once('=') {
                Some((k, v)) if k == "TYPE" => v.split(',').map(|t| t.trim_matches('"').to_string()).collect::<Vec<_>>(),
                Some(_) => vec![],
                None => vec![p.clone()], // vCard 2.1: bare HOME / WORK / CELL
            })
            .collect();
        let qp = params.iter().any(|p| p == "ENCODING=QUOTED-PRINTABLE" || p == "QUOTED-PRINTABLE");
        let utf8 = !params.iter().any(|p| p.starts_with("CHARSET=") && !p.contains("UTF-8"));
        let value: String = if qp { quoted_printable(value, utf8) } else { value.to_string() };
        match name.as_str() {
            "BEGIN" if value.trim().eq_ignore_ascii_case("VCARD") => cur = Some(Imported::default()),
            "END" if value.trim().eq_ignore_ascii_case("VCARD") => {
                if let Some(c) = cur.take().and_then(finish) {
                    out.push(c);
                }
            }
            _ => {
                let Some(c) = cur.as_mut() else { continue };
                match name.as_str() {
                    "FN" => c.name = vunescape(&value).trim().to_string(),
                    "N" => {
                        let p = vsplit(&value);
                        c.surname = none_if_empty(p.first().cloned().unwrap_or_default());
                        c.given = none_if_empty(p.get(1).cloned().unwrap_or_default());
                    }
                    "EMAIL" => add_email(&mut c.emails, &vunescape(&value)),
                    "TEL" => {
                        let kind = ["CELL", "MOBILE", "HOME", "WORK", "FAX", "MAIN", "PAGER"].iter().find(|k| types.iter().any(|t| t == *k)).map(|k| k.to_lowercase()).unwrap_or_else(|| "other".into());
                        let kind = if kind == "cell" { "mobile".to_string() } else { kind };
                        add_phone(&mut c.phones, &kind, &vunescape(value.trim_start_matches("tel:")));
                    }
                    "ORG" => c.company = none_if_empty(vsplit(&value).into_iter().next().unwrap_or_default()),
                    "TITLE" => c.title = none_if_empty(vunescape(&value)),
                    _ => {}
                }
            }
        }
    }
    out
}

/// RFC 4180 CSV: quoted fields, doubled quotes, line breaks inside quotes; rows of cells.
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = vec![];
    let mut row: Vec<String> = vec![];
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    cell.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                cell.push(c);
            }
        } else {
            match c {
                '"' => quoted = true,
                ',' => row.push(std::mem::take(&mut cell)),
                '\r' => {}
                '\n' => {
                    row.push(std::mem::take(&mut cell));
                    rows.push(std::mem::take(&mut row));
                }
                _ => cell.push(c),
            }
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push(row);
    }
    rows.into_iter().filter(|r| r.iter().any(|c| !c.trim().is_empty())).collect()
}

/// Google Contacts CSV, both the classic export ("Name", "E-mail 1 - Value", "Organization 1 - Name" …) and the newer
/// one ("First Name", "Last Name", "E-mail 1 - Label" …); Outlook-style columns ("E-mail Address", "Mobile Phone") too.
pub fn parse_contacts_csv(text: &str) -> Vec<Imported> {
    let rows = parse_csv(text);
    let Some(header) = rows.first() else { return vec![] };
    let header: Vec<String> = header.iter().map(|h| h.trim().trim_start_matches('\u{feff}').to_lowercase()).collect();
    let col = |names: &[&str]| header.iter().position(|h| names.iter().any(|n| h == n));
    let name_col = col(&["name", "display name", "full name"]);
    let given_col = col(&["given name", "first name"]);
    let middle_col = col(&["additional name", "middle name"]);
    let surname_col = col(&["family name", "last name"]);
    let company_col = col(&["organization 1 - name", "organization name", "company"]);
    let title_col = col(&["organization 1 - title", "organization title", "job title"]);
    let email_cols: Vec<usize> = header.iter().enumerate().filter(|(_, h)| h.starts_with("e-mail") && (h.ends_with("value") || h.ends_with("address"))).map(|(i, _)| i).collect();
    // Phone numbers: "Phone 1 - Value" (type/label in the column before it) or "Mobile Phone" / "Home Phone" …
    let phone_cols: Vec<(usize, Option<usize>, String)> = header
        .iter()
        .enumerate()
        .filter_map(|(i, h)| {
            if h.starts_with("phone") && h.ends_with("value") {
                let label = h.trim_end_matches("value").to_string();
                let kind_col = header.iter().position(|x| x.starts_with(&label) && (x.ends_with("type") || x.ends_with("label")));
                Some((i, kind_col, "other".to_string()))
            } else if h.ends_with(" phone") && !h.contains("label") && !h.contains("type") {
                Some((i, None, h.trim_end_matches(" phone").split_whitespace().last().unwrap_or("other").to_string()))
            } else {
                None
            }
        })
        .collect();
    let mut out = vec![];
    for r in rows.iter().skip(1) {
        let cell = |i: Option<usize>| i.and_then(|i| r.get(i)).map(|s| s.trim().to_string()).unwrap_or_default();
        let mut c = Imported { name: cell(name_col), given: none_if_empty(cell(given_col)), surname: none_if_empty(cell(surname_col)), company: none_if_empty(cell(company_col)), title: none_if_empty(cell(title_col)), ..Default::default() };
        if c.name.is_empty() {
            c.name = [cell(given_col), cell(middle_col), cell(surname_col)].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
        }
        for &i in &email_cols {
            for e in cell(Some(i)).split(" ::: ") {
                add_email(&mut c.emails, e);
            }
        }
        for (i, kind_col, fallback) in &phone_cols {
            let kind = kind_col.map(|k| cell(Some(k)).to_lowercase()).filter(|k| !k.is_empty()).unwrap_or_else(|| fallback.clone());
            let kind = kind.trim_start_matches("* ").replace("cell", "mobile");
            for n in cell(Some(*i)).split(" ::: ") {
                add_phone(&mut c.phones, &kind, n);
            }
        }
        if let Some(c) = finish(c) {
            out.push(c);
        }
    }
    out
}

/// Merges entries that share an email address (case-insensitive); entries without one merge by name.
pub fn dedupe(list: Vec<Imported>) -> Vec<Imported> {
    let mut out: Vec<Imported> = vec![];
    let mut by_email: HashMap<String, usize> = HashMap::new();
    let mut by_name: HashMap<String, usize> = HashMap::new();
    for c in list {
        let hit = c.emails.iter().find_map(|e| by_email.get(&e.to_lowercase()).copied()).or_else(|| if c.emails.is_empty() { by_name.get(&c.name.to_lowercase()).copied() } else { None });
        let idx = match hit {
            Some(i) => {
                merge_into(&mut out[i], &c);
                i
            }
            None => {
                out.push(c.clone());
                out.len() - 1
            }
        };
        for e in &out[idx].emails {
            by_email.insert(e.to_lowercase(), idx);
        }
        if out[idx].emails.is_empty() {
            by_name.insert(out[idx].name.to_lowercase(), idx);
        }
    }
    out
}

/// Adds what `b` knows to `a`: new addresses and numbers, and fields `a` was missing.
fn merge_into(a: &mut Imported, b: &Imported) {
    for e in &b.emails {
        add_email(&mut a.emails, e);
    }
    for (k, n) in &b.phones {
        add_phone(&mut a.phones, k, n);
    }
    if a.name.trim().is_empty() || (a.emails.iter().any(|e| e.eq_ignore_ascii_case(&a.name)) && !b.name.trim().is_empty() && !b.emails.iter().any(|e| e.eq_ignore_ascii_case(&b.name))) {
        a.name = b.name.clone();
    }
    for (mine, theirs) in [(&mut a.given, &b.given), (&mut a.surname, &b.surname), (&mut a.company, &b.company), (&mut a.title, &b.title)] {
        if mine.is_none() {
            *mine = theirs.clone();
        }
    }
}

/// Reads a .vcf or .csv file into contacts (format decided by content, then by extension).
pub fn parse_file(path: &str) -> Result<Vec<Imported>> {
    let bytes = std::fs::read(path).map_err(|e| anyhow!("Couldn't read {path}: {e}"))?;
    if bytes.len() > 50 * 1024 * 1024 {
        bail!("That file is too big to be a contacts file (over 50 MB).");
    }
    let text = decode_text(&bytes);
    let head = text.trim_start().get(..11).unwrap_or("").to_ascii_uppercase();
    let is_vcf = head.starts_with("BEGIN:VCARD") || (path.to_ascii_lowercase().ends_with(".vcf") && !head.contains(','));
    let list = if is_vcf { parse_vcards(&text) } else { parse_contacts_csv(&text) };
    if list.is_empty() {
        bail!("No contacts were found in that file. Graphite Mail reads vCard (.vcf) files and Google Contacts CSV exports.");
    }
    Ok(dedupe(list))
}

// ---------------------------------------------------------------- storing imports

#[derive(Serialize, Debug, Default, PartialEq)]
pub struct ImportReport {
    pub added: usize,
    pub updated: usize,
    pub skipped: usize,
}

fn search_text(c: &Imported) -> String {
    let mut s = vec![c.name.to_lowercase()];
    s.extend(c.emails.iter().map(|e| e.to_lowercase()));
    s.extend(c.company.iter().chain(c.title.iter()).map(|x| x.to_lowercase()));
    s.join(" ")
}

/// Saves contacts under an IMAP account as "Imported". One that shares an address with an imported contact already
/// there updates it (addresses and numbers are merged) instead of making a double.
pub fn store_imported(account_id: &str, list: &[Imported]) -> Result<ImportReport> {
    storage::with_db(|db| {
        let tx = db.unchecked_transaction()?;
        let mut rep = ImportReport::default();
        for c in list {
            if c.name.trim().is_empty() && c.emails.is_empty() {
                rep.skipped += 1;
                continue;
            }
            let mut existing: Option<(String, String, String, String, Option<String>, Option<String>, Option<String>, Option<String>)> = None;
            for e in &c.emails {
                existing = tx
                    .query_row(
                        "SELECT id, display_name, emails_json, phones_json, company, job_title, given_name, surname FROM contacts
                         WHERE account_id=?1 AND source=?2 AND instr(lower(emails_json), '\"' || lower(?3) || '\"') > 0 LIMIT 1",
                        params![account_id, IMPORTED, e],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)),
                    )
                    .optional()?;
                if existing.is_some() {
                    break;
                }
            }
            let phones_json = |p: &[(String, String)]| Value::Array(p.iter().map(|(k, n)| json!({ "kind": k, "number": n })).collect()).to_string();
            match existing {
                Some((id, name, emails, phones, company, title, given, surname)) => {
                    let old_phones: Vec<Value> = serde_json::from_str(&phones).unwrap_or_default();
                    let mut merged = Imported {
                        name,
                        given,
                        surname,
                        emails: serde_json::from_str(&emails).unwrap_or_default(),
                        phones: old_phones.iter().map(|p| (p["kind"].as_str().unwrap_or("other").to_string(), p["number"].as_str().unwrap_or_default().to_string())).collect(),
                        company,
                        title,
                    };
                    merge_into(&mut merged, c);
                    tx.execute(
                        "UPDATE contacts SET display_name=?2, given_name=?3, surname=?4, emails_json=?5, phones_json=?6, company=?7, job_title=?8, search_text=?9 WHERE id=?1",
                        params![id, merged.name, merged.given, merged.surname, json!(merged.emails).to_string(), phones_json(&merged.phones), merged.company, merged.title, search_text(&merged)],
                    )?;
                    rep.updated += 1;
                }
                None => {
                    tx.execute(
                        "INSERT INTO contacts(id,account_id,source,remote_id,display_name,given_name,surname,emails_json,phones_json,company,job_title,search_text,sync_gen)
                         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,0)",
                        params![uuid::Uuid::new_v4().to_string(), account_id, IMPORTED, uuid::Uuid::new_v4().to_string(), c.name, c.given, c.surname, json!(c.emails).to_string(), phones_json(&c.phones), c.company, c.title, search_text(c)],
                    )?;
                    rep.added += 1;
                }
            }
        }
        tx.commit()?;
        Ok(rep)
    })
}

/// "Import contacts…": reads the file and stores its contacts under the IMAP account `email`.
pub fn import_file(email: &str, path: &str) -> Result<ImportReport> {
    let account_id = imap_account_id(email)?;
    let list = parse_file(path)?;
    store_imported(&account_id, &list)
}

/// The accounts table id of an IMAP (app password) account.
pub fn imap_account_id(email: &str) -> Result<String> {
    storage::with_db(|db| {
        Ok(db
            .query_row("SELECT id FROM accounts WHERE lower(email)=lower(?1) AND provider=?2", params![email, crate::imap::PROVIDER], |r| r.get(0))
            .optional()?
            .ok_or_else(|| anyhow!("Contacts can be imported into a Gmail (app password) account."))?)
    })
}

#[cfg(test)]
#[path = "local_contacts_tests.rs"]
mod tests;
