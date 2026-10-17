//! Inline (cid:) attachments: find the references in a message body, parse Graph's attachment list,
//! and swap the references for data: URIs when a message is rendered.

use base64::Engine;
use serde_json::Value;

/// Graph's description of one file attachment.
#[derive(Debug, Clone, PartialEq)]
pub struct Att {
    pub remote_id: String,
    pub name: String,
    pub mime: String,
    pub size: i64,
    pub content_id: Option<String>,
    pub inline: bool,
    pub bytes: Option<Vec<u8>>,
}

/// Content-IDs are compared lowercase, percent-decoded and without angle brackets.
pub fn norm_cid(raw: &str) -> String {
    let b = raw.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = ((b[i + 1] as char).to_digit(16), (b[i + 2] as char).to_digit(16)) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).trim().trim_matches(|c| c == '<' || c == '>').to_lowercase()
}

/// Byte ranges (start of "cid:", end of the id) of every cid: reference in `html`.
fn cid_spans(html: &str) -> Vec<(usize, usize)> {
    let lower = html.to_ascii_lowercase(); // same byte offsets as `html`
    let bytes = lower.as_bytes();
    let mut spans = Vec::new();
    let mut from = 0;
    while let Some(p) = lower[from..].find("cid:") {
        let start = from + p;
        from = start + 4;
        if start > 0 && bytes[start - 1].is_ascii_alphanumeric() {
            continue; // e.g. "decid:" is not a reference
        }
        let end = lower[from..]
            .find(|c: char| c == '"' || c == '\'' || c == ')' || c == '<' || c == '>' || c == '\\' || c.is_whitespace())
            .map(|e| from + e)
            .unwrap_or(lower.len());
        if end > from {
            spans.push((start, end));
            from = end;
        }
    }
    spans
}

/// Normalised ids of all cid: references in the body (deduplicated, in order of appearance).
pub fn cid_refs(html: &str) -> Vec<String> {
    let mut seen = Vec::new();
    for (s, e) in cid_spans(html) {
        let id = norm_cid(&html[s + 4..e]);
        if !id.is_empty() && !seen.contains(&id) {
            seen.push(id);
        }
    }
    seen
}

fn safe_image_mime(m: &str) -> bool {
    m.len() < 64 && m.starts_with("image/") && m.bytes().all(|c| c.is_ascii_alphanumeric() || b"/.+-".contains(&c))
}

/// Replaces every cid: reference for which `lookup(normalised id)` returns `(mime, bytes)` with a data: URI.
/// Unknown references are left untouched.
pub fn resolve_cids(html: &str, mut lookup: impl FnMut(&str) -> Option<(String, Vec<u8>)>) -> String {
    let mut out = String::with_capacity(html.len());
    let mut last = 0;
    for (s, e) in cid_spans(html) {
        let id = norm_cid(&html[s + 4..e]);
        if let Some((mime, bytes)) = lookup(&id).filter(|(m, _)| safe_image_mime(m)) {
            out.push_str(&html[last..s]);
            out.push_str(&format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)));
            last = e;
        }
    }
    out.push_str(&html[last..]);
    out
}

/// Parses Graph's `/messages/{id}/attachments` response, keeping file attachments only.
/// `contentBytes` is decoded when Graph included it.
pub fn parse_attachments(v: &Value) -> Vec<Att> {
    v["value"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or_default()
        .iter()
        .filter(|a| a["@odata.type"].as_str().is_none_or(|t| t.ends_with("fileAttachment")))
        .filter_map(|a| {
            Some(Att {
                remote_id: a["id"].as_str()?.to_string(),
                name: a["name"].as_str().unwrap_or("attachment").to_string(),
                mime: a["contentType"].as_str().unwrap_or("application/octet-stream").to_ascii_lowercase(),
                size: a["size"].as_i64().unwrap_or(0),
                content_id: a["contentId"].as_str().map(norm_cid).filter(|c| !c.is_empty()),
                inline: a["isInline"].as_bool().unwrap_or(false),
                bytes: a["contentBytes"]
                    .as_str()
                    .and_then(|b| base64::engine::general_purpose::STANDARD.decode(b.trim()).ok()),
            })
        })
        .collect()
}

impl Att {
    /// True for attachments that a message body can reference with cid:.
    pub fn is_embeddable(&self) -> bool {
        self.content_id.is_some() && (self.inline || self.mime.starts_with("image/"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn finds_references_in_all_common_places() {
        let html = r#"<img src="cid:logo@x"><IMG SRC='CID:Banner.PNG'><div style="background:url(cid:bg1)"></div>
            <img src=cid:bare1 width=3><img src="cid:logo@x"> not a ref: decid:nope, mailto:cid@x.com"#;
        assert_eq!(cid_refs(html), vec!["logo@x", "banner.png", "bg1", "bare1"]);
    }

    #[test]
    fn normalises_ids() {
        assert_eq!(norm_cid("<Image001.PNG@01D9>"), "image001.png@01d9");
        assert_eq!(norm_cid("logo%40x.com"), "logo@x.com");
        assert_eq!(norm_cid("100%"), "100%"); // stray % is kept
        assert_eq!(norm_cid("%4"), "%4");
    }

    #[test]
    fn replaces_known_refs_only() {
        let html = r#"<img src="cid:a"><img src='cid:B%40x'><img src="cid:missing">"#;
        let out = resolve_cids(html, |id| match id {
            "a" => Some(("image/png".into(), vec![1, 2, 3])),
            "b@x" => Some(("image/gif".into(), vec![9])),
            _ => None,
        });
        assert_eq!(
            out,
            r#"<img src="data:image/png;base64,AQID"><img src='data:image/gif;base64,CQ=='><img src="cid:missing">"#
        );
    }

    #[test]
    fn refuses_non_image_or_odd_mime_types() {
        for mime in ["text/html", "image/png;x=\"><script>", "application/pdf", ""] {
            let out = resolve_cids(r#"<img src="cid:a">"#, |_| Some((mime.to_string(), vec![1])));
            assert_eq!(out, r#"<img src="cid:a">"#, "mime {mime:?} must not be embedded");
        }
    }

    #[test]
    fn leaves_html_without_refs_untouched() {
        let html = "<p>héllo wörld ✓</p>";
        assert_eq!(resolve_cids(html, |_| panic!("no lookup expected")), html);
    }

    #[test]
    fn handles_multibyte_text_around_refs() {
        let html = "<p>ünïcödé</p><img src=\"cid:é1\"><p>✓</p>";
        let out = resolve_cids(html, |id| (id == "é1").then(|| ("image/png".into(), vec![0])));
        assert_eq!(out, "<p>ünïcödé</p><img src=\"data:image/png;base64,AA==\"><p>✓</p>");
    }

    #[test]
    fn parses_graph_attachment_list() {
        let v = json!({ "value": [
            { "@odata.type": "#microsoft.graph.fileAttachment", "id": "A1", "name": "logo.png", "contentType": "image/PNG",
              "size": 3, "isInline": true, "contentId": "logo123@mail", "contentBytes": "AQID" },
            { "@odata.type": "#microsoft.graph.fileAttachment", "id": "A2", "name": "report.pdf", "contentType": "application/pdf",
              "size": 10, "isInline": false, "contentId": null },
            { "@odata.type": "#microsoft.graph.itemAttachment", "id": "A3", "name": "forwarded" },
            { "@odata.type": "#microsoft.graph.referenceAttachment", "id": "A4", "name": "link" },
        ]});
        let atts = parse_attachments(&v);
        assert_eq!(atts.len(), 2);
        assert_eq!(atts[0].mime, "image/png");
        assert_eq!(atts[0].content_id.as_deref(), Some("logo123@mail"));
        assert_eq!(atts[0].bytes.as_deref(), Some(&[1u8, 2, 3][..]));
        assert!(atts[0].is_embeddable());
        assert!(!atts[1].is_embeddable());
        assert_eq!(atts[1].bytes, None);
        assert!(parse_attachments(&json!({})).is_empty());
    }
}
