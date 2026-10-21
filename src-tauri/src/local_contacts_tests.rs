//! Local contacts for Gmail (app password) accounts: header extraction, automated-sender rules, the incremental
//! "People you've emailed" rebuild, vCard / CSV parsing, de-duplication, imports, removal and account deletion.

use super::*;
use crate::storage;

fn setup() -> std::path::PathBuf {
    let tmp = std::env::temp_dir().join(format!("gm-local-contacts-{}", uuid::Uuid::new_v4()));
    unsafe { std::env::set_var("XDG_CONFIG_HOME", tmp.join("cfg")) };
    storage::initialize(tmp.join("vault").to_str().unwrap()).unwrap();
    storage::with_db(|db| {
        db.execute_batch(
            "INSERT INTO accounts(id,email,display_name,created_at,provider) VALUES('g','me@gmail.com','Me',1,'imap'),('a','me@corp.com','Work',2,'microsoft');
             INSERT INTO folders(id,account_id,remote_id,display_name,well_known_name) VALUES
               ('inbox','g','INBOX','Inbox','inbox'),('sent','g','[Gmail]/Sent Mail','Sent','sentitems'),('spam','g','[Gmail]/Spam','Spam','junkemail'),('bin','g','[Gmail]/Bin','Bin','deleteditems');",
        )?;
        Ok(())
    })
    .unwrap();
    tmp
}

fn msg(id: &str, folder: &str, from: (&str, &str), to: &[(&str, &str)], cc: &[(&str, &str)], at: i64) {
    let people = |l: &[(&str, &str)]| Value::Array(l.iter().map(|(n, a)| json!({ "name": if n.is_empty() { Value::Null } else { json!(n) }, "address": a })).collect()).to_string();
    storage::with_db(|db| {
        db.execute(
            "INSERT INTO messages(id,account_id,folder_id,remote_id,sender_name,sender_address,to_recipients_json,cc_recipients_json,received_at) VALUES(?1,'g',?2,?1,?3,?4,?5,?6,?7)",
            params![id, folder, if from.0.is_empty() { None } else { Some(from.0) }, from.1, people(to), people(cc), at],
        )?;
        Ok(())
    })
    .unwrap();
}

fn emailed() -> Vec<(String, String, i64, Option<i64>)> {
    storage::with_db(|db| {
        let mut st = db.prepare("SELECT remote_id, display_name, message_count, last_contacted FROM contacts WHERE account_id='g' AND source='emailed' ORDER BY remote_id")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    })
    .unwrap()
}

#[test]
fn automated_senders_are_recognised_and_people_are_not() {
    for a in [
        "noreply@github.com", "no-reply@accounts.google.com", "donotreply@bank.com", "mailer-daemon@googlemail.com", "postmaster@x.org",
        "notifications@github.com", "notification+abc@facebookmail.com", "newsletter@shop.com", "alerts@monitor.io", "bounce-12ab34cd56ef7890abcd@em.example.com",
        "dev-list-request@lists.example.org", "owner-dev@lists.example.org", "unsubscribe@news.example.com", "bot@slack.com", "not-an-address", "x@localhost",
        "01234567890abcdef0123456789abcdef@bounce.example.com",
    ] {
        assert!(is_automated(a, None), "{a} is automated");
    }
    for a in ["ana@gmail.com", "ben.ode@corp.com", "info@smallshop.com", "support@startup.io", "notify.jane@gmail.com", "robert@bot.engineering", "a.b+work@x.com"] {
        assert!(!is_automated(a, None), "{a} is a person");
    }
    assert!(is_automated("jane@x.com", Some("Jane (no-reply)")), "the display name can give it away");
    assert!(is_automated("x@y.com", Some("Mail Delivery Subsystem")));
}

#[test]
fn people_come_from_sender_to_and_cc() {
    let p = people_in(Some("\"Ana Diaz\""), Some("ana@x.com"), Some(r#"[{"name":"Me","address":"me@gmail.com"},{"name":null,"address":"ben@x.com"}]"#), Some(r#"[{"name":"","address":"cc@x.com"},{"address":"broken"}]"#));
    assert_eq!(
        p,
        vec![
            Person { name: Some("Ana Diaz".into()), address: "ana@x.com".into() },
            Person { name: Some("Me".into()), address: "me@gmail.com".into() },
            Person { name: None, address: "ben@x.com".into() },
            Person { name: None, address: "cc@x.com".into() },
        ]
    );
    assert!(people_in(None, None, Some("not json"), None).is_empty());
    assert_eq!(people_in(Some("ana@x.com"), Some("ana@x.com"), None, None)[0].name, None, "an address is not a name");
}

#[test]
fn people_you_have_emailed_are_built_incrementally() {
    let _g = storage::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    setup();
    msg("m1", "inbox", ("Ana Diaz", "ana@x.com"), &[("Me", "me@gmail.com")], &[("", "ben@x.com")], 100);
    msg("m2", "sent", ("Me", "me@gmail.com"), &[("Ben Ode", "ben@x.com"), ("Ben Ode", "BEN@x.com")], &[], 200);
    msg("m3", "inbox", ("GitHub", "notifications@github.com"), &[("Me", "me@gmail.com")], &[], 300);
    msg("m4", "spam", ("Spammer", "spam@x.com"), &[("Me", "me@gmail.com")], &[], 400);
    msg("m5", "bin", ("Old", "old@x.com"), &[("Me", "me@gmail.com")], &[], 500);
    assert_eq!(rebuild_people("g").unwrap(), 2);
    assert_eq!(
        emailed(),
        vec![("ana@x.com".to_string(), "Ana Diaz".to_string(), 1, Some(100)), ("ben@x.com".to_string(), "Ben Ode".to_string(), 2, Some(200))],
        "own address, spam, bin and automated senders are left out; the same address twice in one message counts once; the name comes from the newest message that had one"
    );
    // Nothing new: nothing changes, nothing double-counted.
    assert_eq!(rebuild_people("g").unwrap(), 0);
    assert_eq!(emailed()[1].2, 2);
    // New mail after a sync: only it is read; counts and names update.
    msg("m6", "inbox", ("Benjamin Ode", "ben@x.com"), &[("Me", "me@gmail.com"), ("Cara", "cara@x.com")], &[], 600);
    assert_eq!(rebuild_people("g").unwrap(), 2);
    let e = emailed();
    assert_eq!((e[1].1.as_str(), e[1].2, e[1].3), ("Benjamin Ode", 3, Some(600)));
    assert_eq!((e[2].0.as_str(), e[2].1.as_str(), e[2].2), ("cara@x.com", "Cara", 1));
    // Deleting old mail is noticed: a full rebuild keeps the counts honest.
    storage::with_db(|db| Ok(db.execute("DELETE FROM messages WHERE id='m2'", [])?)).unwrap();
    rebuild_people("g").unwrap();
    assert_eq!(emailed()[1].2, 2, "m2 no longer counts for Ben");
    // The overview lists them (newest first), the detail knows the counts, suggestions work for the Gmail account.
    let o = crate::contacts::overview("", Some("me@gmail.com")).unwrap();
    assert_eq!(o.emailed.rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), vec!["Benjamin Ode", "Cara", "Ana Diaz"]);
    assert_eq!((o.emailed.rows[0].message_count, o.emailed.rows[0].last_contacted), (Some(2), Some(600)));
    assert_eq!((o.directory.total, o.personal.total, o.imported.total), (0, 0, 0));
    let d = crate::contacts::detail(&o.emailed.rows[0].id).unwrap();
    assert_eq!((d.source.as_str(), d.emails.clone(), d.message_count), ("emailed", vec!["ben@x.com".to_string()], Some(2)));
    let s = crate::contacts::suggest("ben", "me@gmail.com", 8).unwrap();
    assert_eq!(s.iter().map(|x| x.email.as_str()).collect::<Vec<_>>(), vec!["ben@x.com"]);
    assert!(crate::contacts::suggest("ben", "me@corp.com", 8).unwrap().is_empty(), "another account's people stay private to it");
    // Favourites work on them, and survive a full rebuild (keyed by address).
    crate::contacts::set_favourite(&o.emailed.rows[1].id, true).unwrap();
    storage::with_db(|db| Ok(db.execute("DELETE FROM messages WHERE id='m1'", [])?)).unwrap();
    rebuild_people("g").unwrap();
    let o = crate::contacts::overview("", Some("me@gmail.com")).unwrap();
    assert_eq!(o.favourites.rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), vec!["Cara"]);
    // "Remove cached contacts": gone and not rebuilt in the background; "Refresh contacts" brings them back.
    crate::contacts::remove_cached_for("me@gmail.com").unwrap();
    assert!(emailed().is_empty());
    assert_eq!(rebuild_people("g").unwrap(), 0, "paused");
    assert!(emailed().is_empty());
    refresh_people("g").unwrap();
    assert_eq!(emailed().len(), 2, "rebuilt from the mail still here (m6: Ben and Cara)");
}

const VCF_30: &str = "BEGIN:VCARD\r\nVERSION:3.0\r\nN:Diaz;Ana;;;\r\nFN:Ana Diaz\r\nORG:Contoso;Finance\r\nTITLE:CFO\r\nEMAIL;TYPE=INTERNET,WORK:ana@corp.com\r\nitem1.EMAIL;TYPE=INTERNET:ana.diaz@home.com\r\nTEL;TYPE=CELL:+1 555 0100\r\nTEL;TYPE=WORK,VOICE:+1 555 0101\r\nNOTE:Long note that is folded over\r\n  two lines\\, with an escaped comma\r\nEND:VCARD\r\nBEGIN:VCARD\r\nVERSION:4.0\r\nFN:Ben Ode\r\nEMAIL:ben@x.com\r\nTEL;VALUE=uri;TYPE=home:tel:+44 20 7946 0000\r\nEND:VCARD\r\n";

#[test]
fn vcards_2_1_3_0_and_4_0_are_read_with_folding_quoted_printable_and_several_addresses() {
    let v = parse_vcards(VCF_30);
    assert_eq!(v.len(), 2);
    assert_eq!((v[0].name.as_str(), v[0].given.as_deref(), v[0].surname.as_deref(), v[0].company.as_deref(), v[0].title.as_deref()), ("Ana Diaz", Some("Ana"), Some("Diaz"), Some("Contoso"), Some("CFO")));
    assert_eq!(v[0].emails, vec!["ana@corp.com", "ana.diaz@home.com"]);
    assert_eq!(v[0].phones, vec![("mobile".to_string(), "+1 555 0100".to_string()), ("work".to_string(), "+1 555 0101".to_string())]);
    assert_eq!((v[1].name.as_str(), v[1].emails[0].as_str(), v[1].phones[0].0.as_str(), v[1].phones[0].1.as_str()), ("Ben Ode", "ben@x.com", "home", "+44 20 7946 0000"));

    // vCard 2.1 from an old phone: quoted-printable Latin-1 name, bare type words, LF line ends, soft line break.
    let v21 = "BEGIN:VCARD\nVERSION:2.1\nN;CHARSET=ISO-8859-1;ENCODING=QUOTED-PRINTABLE:Fran=E7ois;Jos=E9\nFN;CHARSET=ISO-8859-1;ENCODING=QUOTED-PRINTABLE:Jos=E9 Fran=E7ois =\nde la Cruz\nEMAIL;PREF;INTERNET:jose@x.com\nTEL;CELL:0600000000\nEND:VCARD\n";
    let v = parse_vcards(v21);
    assert_eq!(v.len(), 1);
    assert_eq!((v[0].name.as_str(), v[0].given.as_deref(), v[0].surname.as_deref()), ("José François de la Cruz", Some("José"), Some("François")));
    assert_eq!(v[0].phones, vec![("mobile".to_string(), "0600000000".to_string())]);
    // UTF-8 quoted-printable too.
    let v = parse_vcards("BEGIN:VCARD\r\nVERSION:2.1\r\nFN;ENCODING=QUOTED-PRINTABLE;CHARSET=UTF-8:Z=C3=B6e\r\nEMAIL:zoe@x.com\r\nEND:VCARD\r\n");
    assert_eq!(v[0].name, "Zöe");
    // No FN: the name parts, else the address. Nothing at all: skipped.
    let v = parse_vcards("BEGIN:VCARD\r\nN:Ode;Ben\r\nEMAIL:b@x.com\r\nEND:VCARD\r\nBEGIN:VCARD\r\nEMAIL:only@x.com\r\nEND:VCARD\r\nBEGIN:VCARD\r\nNOTE:nothing\r\nEND:VCARD\r\n");
    assert_eq!(v.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), vec!["Ben Ode", "only@x.com"]);
}

#[test]
fn odd_encodings_are_decoded() {
    assert_eq!(decode_text("\u{feff}FN:Zoë".as_bytes()), "FN:Zoë", "UTF-8 BOM dropped");
    assert_eq!(decode_text(b"FN:Zo\xEB"), "FN:Zoë", "Latin-1");
    assert_eq!(decode_text(b"caf\xE9 \x93quoted\x94 \x96 \x80"), "café “quoted” – €", "Windows-1252 punctuation");
    let mut le = vec![0xFF, 0xFE];
    for u in "FN:Zoë".encode_utf16() {
        le.extend_from_slice(&u.to_le_bytes());
    }
    assert_eq!(decode_text(&le), "FN:Zoë", "UTF-16 LE with BOM (Outlook CSV)");
    let mut be = vec![0xFE, 0xFF];
    for u in "Ab".encode_utf16() {
        be.extend_from_slice(&u.to_be_bytes());
    }
    assert_eq!(decode_text(&be), "Ab");
}

const GOOGLE_OLD: &str = "\u{feff}Name,Given Name,Additional Name,Family Name,Organization 1 - Name,Organization 1 - Title,E-mail 1 - Type,E-mail 1 - Value,E-mail 2 - Type,E-mail 2 - Value,Phone 1 - Type,Phone 1 - Value\r\n\
Ana Diaz,Ana,,Diaz,Contoso,CFO,* Work,ana@corp.com,Home,ana.diaz@home.com ::: ana2@home.com,Mobile,+1 555 0100\r\n\
\"Ode, Ben\",Ben,,Ode,,,,ben@x.com,,,Work,+1 555 0101 ::: +1 555 0102\r\n\
,,,,,,,,,,,\r\n\
\"Multi\nLine\",,,,\"Quotes \"\"Inc\"\"\",,,multi@x.com,,,,\r\n";

const GOOGLE_NEW: &str = "First Name,Middle Name,Last Name,Phonetic First Name,Organization Name,Organization Title,E-mail 1 - Label,E-mail 1 - Value,Phone 1 - Label,Phone 1 - Value\n\
Ana,,Diaz,,Contoso,CFO,Work,ana@corp.com,Mobile,+1 555 0100\n\
,,,,,,,nobody@x.com,,\n";

#[test]
fn google_contacts_csv_old_and_new_formats_are_read() {
    let v = parse_contacts_csv(GOOGLE_OLD);
    assert_eq!(v.len(), 3, "the empty row is skipped: {v:?}");
    assert_eq!((v[0].name.as_str(), v[0].given.as_deref(), v[0].surname.as_deref(), v[0].company.as_deref(), v[0].title.as_deref()), ("Ana Diaz", Some("Ana"), Some("Diaz"), Some("Contoso"), Some("CFO")));
    assert_eq!(v[0].emails, vec!["ana@corp.com", "ana.diaz@home.com", "ana2@home.com"], "' ::: ' separates several values in one cell");
    assert_eq!(v[0].phones, vec![("mobile".to_string(), "+1 555 0100".to_string())]);
    assert_eq!((v[1].name.as_str(), v[1].emails[0].as_str()), ("Ode, Ben", "ben@x.com"));
    assert_eq!(v[1].phones, vec![("work".to_string(), "+1 555 0101".to_string()), ("work".to_string(), "+1 555 0102".to_string())]);
    assert_eq!((v[2].name.as_str(), v[2].company.as_deref()), ("Multi\nLine", Some("Quotes \"Inc\"")));

    let v = parse_contacts_csv(GOOGLE_NEW);
    assert_eq!(v.len(), 2);
    assert_eq!((v[0].name.as_str(), v[0].company.as_deref(), v[0].emails[0].as_str(), v[0].phones[0].0.as_str()), ("Ana Diaz", Some("Contoso"), "ana@corp.com", "mobile"));
    assert_eq!(v[1].name, "nobody@x.com", "no name at all: the address stands in");

    // Outlook-style export.
    let v = parse_contacts_csv("First Name,Last Name,Company,Job Title,E-mail Address,E-mail 2 Address,Mobile Phone,Business Phone\r\nBen,Ode,Fabrikam,CTO,ben@x.com,ben@home.com,0600,0601\r\n");
    assert_eq!((v[0].name.as_str(), v[0].emails.len(), v[0].phones.clone()), ("Ben Ode", 2, vec![("mobile".to_string(), "0600".to_string()), ("business".to_string(), "0601".to_string())]));
    assert!(parse_contacts_csv("").is_empty() && parse_contacts_csv("just,a,header\n").is_empty());
}

#[test]
fn duplicates_merge_by_address() {
    let c = |name: &str, emails: &[&str]| Imported { name: name.into(), emails: emails.iter().map(|e| e.to_string()).collect(), ..Default::default() };
    let d = dedupe(vec![
        c("Ana Diaz", &["ana@corp.com"]),
        c("A. Diaz", &["ANA@corp.com", "ana@home.com"]),
        Imported { name: "ana@home.com".into(), emails: vec!["ana@home.com".into()], phones: vec![("mobile".into(), "0600".into())], company: Some("Contoso".into()), ..Default::default() },
        c("Ben Ode", &["ben@x.com"]),
        c("Nameless", &[]),
        c("nameless", &[]),
    ]);
    assert_eq!(d.len(), 3, "{d:?}");
    assert_eq!((d[0].name.as_str(), d[0].emails.clone(), d[0].phones.len(), d[0].company.as_deref()), ("Ana Diaz", vec!["ana@corp.com".to_string(), "ana@home.com".to_string()], 1, Some("Contoso")));
    assert_eq!(d[2].name, "Nameless");
}

#[test]
fn imports_are_stored_once_updated_on_reimport_removed_and_deleted_with_the_account() {
    let _g = storage::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = setup();
    let vcf = tmp.join("people.vcf");
    std::fs::write(&vcf, VCF_30).unwrap();
    let csv = tmp.join("google.csv");
    std::fs::write(&csv, GOOGLE_OLD).unwrap();
    assert!(import_file("me@corp.com", vcf.to_str().unwrap()).unwrap_err().to_string().contains("app password"), "only into Gmail (app password) accounts");
    assert_eq!(import_file("me@gmail.com", vcf.to_str().unwrap()).unwrap(), ImportReport { added: 2, updated: 0, skipped: 0 });
    // The same file again makes no doubles; the CSV merges Ana and Ben (shared addresses) and adds the third person.
    assert_eq!(import_file("me@gmail.com", vcf.to_str().unwrap()).unwrap(), ImportReport { added: 0, updated: 2, skipped: 0 });
    assert_eq!(import_file("me@gmail.com", csv.to_str().unwrap()).unwrap(), ImportReport { added: 1, updated: 2, skipped: 0 });
    let o = crate::contacts::overview("", Some("me@gmail.com")).unwrap();
    assert_eq!(o.imported.total, 3);
    assert_eq!(o.imported.rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), vec!["Ana Diaz", "Ben Ode", "Multi\nLine"], "the vCard's name is kept for Ben");
    let ana = o.imported.rows.iter().find(|r| r.name == "Ana Diaz").unwrap();
    let d = crate::contacts::detail(&ana.id).unwrap();
    assert_eq!(d.emails, vec!["ana@corp.com", "ana.diaz@home.com", "ana2@home.com"], "addresses from both files, no doubles");
    assert_eq!(d.phones.len(), 2);
    assert_eq!((d.company.as_deref(), d.job_title.as_deref(), d.source.as_str()), (Some("Contoso"), Some("CFO"), "imported"));
    assert_eq!(crate::contacts::overview("quotes", Some("me@gmail.com")).unwrap().imported.total, 1, "searchable by company");
    let s = crate::contacts::suggest("an", "me@gmail.com", 8).unwrap();
    assert_eq!(s[0].email, "ana@corp.com");
    // Export as vCard works on imported contacts too.
    assert!(crate::contacts::vcards(&[ana.id.clone()]).unwrap().contains("EMAIL;TYPE=INTERNET:ana2@home.com"));
    // Bad files.
    std::fs::write(tmp.join("empty.csv"), "a,b\n").unwrap();
    assert!(import_file("me@gmail.com", tmp.join("empty.csv").to_str().unwrap()).unwrap_err().to_string().contains("No contacts"));
    assert!(import_file("me@gmail.com", tmp.join("missing.vcf").to_str().unwrap()).is_err());
    // Remove cached contacts clears imports; account removal deletes everything of that account.
    crate::contacts::remove_cached_for("me@gmail.com").unwrap();
    assert_eq!(crate::contacts::overview("", Some("me@gmail.com")).unwrap().imported.total, 0);
    import_file("me@gmail.com", vcf.to_str().unwrap()).unwrap();
    crate::contacts::set_favourite(&crate::contacts::overview("", None).unwrap().imported.rows[0].id, true).unwrap();
    crate::sync::forget_account_data("me@gmail.com").unwrap();
    let (c, f): (i64, i64) = storage::with_db(|db| Ok((db.query_row("SELECT COUNT(*) FROM contacts", [], |r| r.get(0))?, db.query_row("SELECT COUNT(*) FROM contact_favourites", [], |r| r.get(0))?))).unwrap();
    assert_eq!((c, f), (0, 0));
}
