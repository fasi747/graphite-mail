//! Calendar: sync a rolling window of events per calendar, serve them offline, queue RSVP/create/delete.

use crate::{auth, storage};
use anyhow::{anyhow, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const GRAPH: &str = "https://graph.microsoft.com/v1.0";
const PAST_DAYS: i64 = 90;
const FUTURE_DAYS: i64 = 365;

async fn get(url: &str) -> Result<Value> {
    let token = auth::access_token().await?;
    let resp = reqwest::Client::new()
        .get(url)
        .bearer_auth(token)
        .header("Prefer", "outlook.timezone=\"UTC\", odata.maxpagesize=200")
        .send()
        .await?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(anyhow!("Graph {status}: {}", v["error"]["message"].as_str().unwrap_or("")));
    }
    Ok(v)
}

/// Graph returns "2026-10-17T09:00:00.0000000" in UTC (because of the Prefer header).
fn parse_utc(v: &Value) -> i64 {
    v["dateTime"]
        .as_str()
        .and_then(|s| chrono::NaiveDateTime::parse_from_str(&s[..19.min(s.len())], "%Y-%m-%dT%H:%M:%S").ok())
        .map(|d| d.and_utc().timestamp())
        .unwrap_or(0)
}

const PALETTE: [&str; 8] = ["#0a84ff", "#ff9f0a", "#30d158", "#bf5af2", "#ff375f", "#64d2ff", "#ffd60a", "#ac8e68"];

/// Graph colour names → hex (hexColor is often empty).
fn color_for(c: &Value, i: usize) -> String {
    if let Some(h) = c["hexColor"].as_str().filter(|h| h.len() == 7) {
        return h.to_string();
    }
    match c["color"].as_str().unwrap_or("auto") {
        "lightBlue" => "#0a84ff", "lightGreen" => "#30d158", "lightOrange" => "#ff9f0a", "lightGray" => "#8e8e93",
        "lightYellow" => "#ffd60a", "lightTeal" => "#64d2ff", "lightPink" => "#ff375f", "lightBrown" => "#ac8e68",
        "lightRed" => "#ff453a",
        _ => PALETTE[i % PALETTE.len()],
    }
    .to_string()
}

pub async fn sync() -> Result<usize> {
    let account_id: String = storage::with_db(|db| {
        Ok(db
            .query_row("SELECT id FROM accounts ORDER BY created_at LIMIT 1", [], |r| r.get(0))
            .optional()?
            .ok_or(anyhow!("No account"))?)
    })?;
    let cals = get(&format!("{GRAPH}/me/calendars?$top=100")).await?;
    let mut total = 0;
    for (i, c) in cals["value"].as_array().cloned().unwrap_or_default().iter().enumerate() {
        let rid = c["id"].as_str().unwrap_or_default().to_string();
        let cal_id: String = storage::with_db(|db| {
            db.execute(
                "INSERT INTO calendars(id,account_id,remote_id,name,color_hex,is_primary,can_edit) VALUES(?1,?2,?3,?4,?5,?6,?7)
                 ON CONFLICT(remote_id) DO UPDATE SET name=excluded.name, color_hex=excluded.color_hex,
                   is_primary=excluded.is_primary, can_edit=excluded.can_edit",
                params![
                    uuid::Uuid::new_v4().to_string(), account_id, rid, c["name"].as_str().unwrap_or("Calendar"),
                    color_for(c, i), c["isDefaultCalendar"].as_bool().unwrap_or(false), c["canEdit"].as_bool().unwrap_or(true)
                ],
            )?;
            Ok(db.query_row("SELECT id FROM calendars WHERE remote_id=?1", [&rid], |r| r.get(0))?)
        })?;
        total += sync_window(&cal_id, &rid).await?;
    }
    Ok(total)
}

/// Full refresh of the rolling window; events no longer returned are removed (unless they have local edits pending).
async fn sync_window(cal_id: &str, remote: &str) -> Result<usize> {
    let now = chrono::Utc::now();
    let start = (now - chrono::Duration::days(PAST_DAYS)).format("%Y-%m-%dT00:00:00Z");
    let end = (now + chrono::Duration::days(FUTURE_DAYS)).format("%Y-%m-%dT00:00:00Z");
    let mut url = format!(
        "{GRAPH}/me/calendars/{remote}/calendarView?startDateTime={start}&endDateTime={end}\
         &$select=id,iCalUId,seriesMasterId,type,subject,body,location,onlineMeeting,start,end,originalStartTimeZone,isAllDay,organizer,attendees,responseStatus,changeKey,isCancelled"
    );
    let mut seen = Vec::new();
    loop {
        let page = get(&url).await?;
        let items = page["value"].as_array().cloned().unwrap_or_default();
        storage::with_db(|db| {
            let tx = db.unchecked_transaction()?;
            for e in &items {
                if e["isCancelled"].as_bool() == Some(true) {
                    continue;
                }
                let rid = e["id"].as_str().unwrap_or_default();
                let attendees: Vec<Value> = e["attendees"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|x| json!({ "name": x["emailAddress"]["name"], "email": x["emailAddress"]["address"],
                                              "response": x["status"]["response"], "type": x["type"] }))
                            .collect()
                    })
                    .unwrap_or_default();
                tx.execute(
                    "INSERT INTO calendar_events(id,calendar_id,remote_id,ical_uid,series_master_id,event_type,title,description_html,location,
                        online_meeting_url,start_utc,end_utc,original_timezone,is_all_day,organizer_json,attendees_json,my_response_status,change_key)
                     VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)
                     ON CONFLICT(remote_id) DO UPDATE SET title=excluded.title, description_html=excluded.description_html,
                        location=excluded.location, online_meeting_url=excluded.online_meeting_url, start_utc=excluded.start_utc,
                        end_utc=excluded.end_utc, is_all_day=excluded.is_all_day, attendees_json=excluded.attendees_json,
                        my_response_status=excluded.my_response_status, change_key=excluded.change_key, calendar_id=excluded.calendar_id
                     WHERE calendar_events.sync_status='SYNCED'",
                    params![
                        uuid::Uuid::new_v4().to_string(), cal_id, rid, e["iCalUId"].as_str().unwrap_or(rid),
                        e["seriesMasterId"].as_str(), e["type"].as_str().unwrap_or("singleInstance"),
                        e["subject"].as_str().unwrap_or("(no title)"), e["body"]["content"].as_str(),
                        e["location"]["displayName"].as_str().filter(|s| !s.is_empty()),
                        e["onlineMeeting"]["joinUrl"].as_str(), parse_utc(&e["start"]), parse_utc(&e["end"]),
                        e["originalStartTimeZone"].as_str().unwrap_or("UTC"), e["isAllDay"].as_bool().unwrap_or(false),
                        e["organizer"]["emailAddress"].to_string(), Value::Array(attendees).to_string(),
                        e["responseStatus"]["response"].as_str().unwrap_or("none"), e["changeKey"].as_str()
                    ],
                )?;
            }
            tx.commit()?;
            Ok(())
        })?;
        seen.extend(items.iter().filter_map(|e| e["id"].as_str().map(String::from)));
        match page["@odata.nextLink"].as_str() {
            Some(n) => url = n.to_string(),
            None => break,
        }
    }
    storage::with_db(|db| {
        let keep: std::collections::HashSet<_> = seen.iter().collect();
        let mut st = db.prepare("SELECT id, remote_id FROM calendar_events WHERE calendar_id=?1 AND sync_status='SYNCED' AND remote_id IS NOT NULL")?;
        let stale: Vec<String> = st
            .query_map([cal_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .filter_map(|r| r.ok())
            .filter(|(_, rid)| !keep.contains(rid))
            .map(|(id, _)| id)
            .collect();
        for id in &stale {
            db.execute("DELETE FROM calendar_events WHERE id=?1", [id])?;
        }
        Ok(())
    })?;
    Ok(seen.len())
}

#[derive(Serialize)]
pub struct Calendar {
    id: String,
    name: String,
    color: String,
    is_primary: bool,
    can_edit: bool,
}

#[derive(Serialize)]
pub struct Event {
    id: String,
    calendar_id: String,
    title: String,
    description_html: Option<String>,
    location: Option<String>,
    online_meeting_url: Option<String>,
    start_utc: i64,
    end_utc: i64,
    is_all_day: bool,
    organizer_json: Option<String>,
    attendees_json: Option<String>,
    my_response: String,
    color: String,
    pending: bool,
}

pub fn calendars() -> Result<Vec<Calendar>> {
    storage::with_db(|db| {
        let mut st = db.prepare("SELECT id,name,color_hex,is_primary,can_edit FROM calendars ORDER BY is_primary DESC, name")?;
        let rows = st.query_map([], |r| {
            Ok(Calendar { id: r.get(0)?, name: r.get(1)?, color: r.get(2)?, is_primary: r.get(3)?, can_edit: r.get(4)? })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    })
}

pub fn events(start: i64, end: i64) -> Result<Vec<Event>> {
    storage::with_db(|db| {
        let mut st = db.prepare(
            "SELECT e.id,e.calendar_id,e.title,e.description_html,e.location,e.online_meeting_url,e.start_utc,e.end_utc,e.is_all_day,
                    e.organizer_json,e.attendees_json,e.my_response_status,c.color_hex,e.sync_status<>'SYNCED'
             FROM calendar_events e JOIN calendars c ON c.id=e.calendar_id
             WHERE e.end_utc > ?1 AND e.start_utc < ?2 ORDER BY e.is_all_day DESC, e.start_utc",
        )?;
        let rows = st.query_map(params![start, end], |r| {
            Ok(Event {
                id: r.get(0)?, calendar_id: r.get(1)?, title: r.get(2)?, description_html: r.get(3)?, location: r.get(4)?,
                online_meeting_url: r.get(5)?, start_utc: r.get(6)?, end_utc: r.get(7)?, is_all_day: r.get(8)?,
                organizer_json: r.get(9)?, attendees_json: r.get(10)?, my_response: r.get(11)?, color: r.get(12)?, pending: r.get(13)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    })
}

fn enqueue(db: &rusqlite::Connection, id: &str, op: &str, payload: Value) -> Result<()> {
    db.execute(
        "INSERT INTO offline_mutation_queue(account_id,entity_type,local_entity_id,operation,payload_json,created_at)
         VALUES((SELECT id FROM accounts ORDER BY created_at LIMIT 1),'CALENDAR_EVENT',?1,?2,?3,?4)",
        params![id, op, payload.to_string(), chrono::Utc::now().timestamp()],
    )?;
    Ok(())
}

/// response: "accept" | "tentativelyAccept" | "decline"
pub fn rsvp(id: &str, response: &str) -> Result<()> {
    let status = match response {
        "accept" => "accepted",
        "tentativelyAccept" => "tentativelyAccepted",
        "decline" => "declined",
        _ => return Err(anyhow!("Unknown response")),
    };
    storage::with_db(|db| {
        // Graph rejects responses to events you organise, to events without invitees, and in read-only calendars
        // (always 400/403), so don't queue those.
        let (mine, attendees, can_edit): (String, Option<String>, bool) = db.query_row(
            "SELECT e.my_response_status, e.attendees_json, c.can_edit FROM calendar_events e JOIN calendars c ON c.id=e.calendar_id WHERE e.id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let invitees = attendees.and_then(|a| serde_json::from_str::<Vec<Value>>(&a).ok()).map_or(0, |a| a.len());
        if mine == "organizer" || invitees == 0 || !can_edit {
            return Err(anyhow!("You can't respond to this event (you organise it, it has no invitees, or the calendar is read-only)"));
        }
        let tx = db.unchecked_transaction()?;
        tx.execute("UPDATE calendar_events SET my_response_status=?1, sync_status='DIRTY_LOCAL' WHERE id=?2", params![status, id])?;
        tx.execute("DELETE FROM offline_mutation_queue WHERE local_entity_id=?1 AND operation='RSVP'", [id])?;
        enqueue(&tx, id, "RSVP", json!({ "response": response }))?;
        tx.commit()?;
        Ok(())
    })
}

#[derive(Deserialize)]
pub struct NewEvent {
    pub calendar_id: String,
    pub title: String,
    pub start_utc: i64,
    pub end_utc: i64,
    pub is_all_day: bool,
    pub location: Option<String>,
    pub attendees: Vec<String>,
    pub body: Option<String>,
    pub online_meeting: bool,
}

pub fn create(e: NewEvent) -> Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    storage::with_db(|db| {
        let tx = db.unchecked_transaction()?;
        let cal_remote: String = tx.query_row("SELECT remote_id FROM calendars WHERE id=?1", [&e.calendar_id], |r| r.get(0))?;
        let attendees: Vec<Value> = e.attendees.iter().map(|a| json!({ "email": a, "response": "none" })).collect();
        tx.execute(
            "INSERT INTO calendar_events(id,calendar_id,ical_uid,event_type,title,description_html,location,start_utc,end_utc,
               original_timezone,is_all_day,attendees_json,my_response_status,sync_status)
             VALUES(?1,?2,?1,'SINGLE',?3,?4,?5,?6,?7,'UTC',?8,?9,'organizer','DIRTY_LOCAL')",
            params![id, e.calendar_id, e.title, e.body, e.location, e.start_utc, e.end_utc, e.is_all_day, Value::Array(attendees).to_string()],
        )?;
        let fmt = |t: i64| chrono::DateTime::from_timestamp(t, 0).unwrap_or_default().format("%Y-%m-%dT%H:%M:%S").to_string();
        enqueue(
            &tx,
            &id,
            "CREATE_EVENT",
            json!({
                "calendar": cal_remote,
                "event": {
                    "subject": e.title,
                    "body": { "contentType": "HTML", "content": e.body.clone().unwrap_or_default() },
                    "start": { "dateTime": fmt(e.start_utc), "timeZone": "UTC" },
                    "end": { "dateTime": fmt(e.end_utc), "timeZone": "UTC" },
                    "isAllDay": e.is_all_day,
                    "location": { "displayName": e.location.clone().unwrap_or_default() },
                    "attendees": e.attendees.iter().map(|a| json!({ "emailAddress": { "address": a }, "type": "required" })).collect::<Vec<_>>(),
                    "isOnlineMeeting": e.online_meeting,
                    "onlineMeetingProvider": if e.online_meeting { "teamsForBusiness" } else { "unknown" },
                }
            }),
        )?;
        tx.commit()?;
        Ok(id)
    })
}

pub fn delete(id: &str) -> Result<()> {
    storage::with_db(|db| {
        let tx = db.unchecked_transaction()?;
        let rid: Option<String> = tx.query_row("SELECT remote_id FROM calendar_events WHERE id=?1", [id], |r| r.get(0))?;
        tx.execute("DELETE FROM calendar_events WHERE id=?1", [id])?;
        match rid {
            Some(r) => enqueue(&tx, id, "DELETE_EVENT", json!({ "remote_id": r }))?,
            None => {
                // Never reached the server: cancel the queued create instead.
                tx.execute("DELETE FROM offline_mutation_queue WHERE local_entity_id=?1", [id])?;
            }
        }
        tx.commit()?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_graph_utc_times() {
        assert_eq!(parse_utc(&json!({ "dateTime": "2026-10-17T09:30:00.0000000", "timeZone": "UTC" })), 1792229400);
        assert_eq!(parse_utc(&json!({})), 0);
    }
    #[test]
    fn maps_colors() {
        assert_eq!(color_for(&json!({ "color": "lightGreen" }), 0), "#30d158");
        assert_eq!(color_for(&json!({ "hexColor": "#123456" }), 0), "#123456");
        assert_eq!(color_for(&json!({ "color": "auto" }), 1), "#ff9f0a");
    }

    #[test]
    fn only_queues_rsvps_graph_can_accept() {
        let _g = storage::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        storage::init_test_vault();
        storage::with_db(|db| {
            db.execute_batch(
                "INSERT INTO accounts VALUES('a','me@x.com','Me',0);
                 INSERT INTO calendars(id,account_id,remote_id,name,color_hex,can_edit) VALUES('c','a','RC','Calendar','#000',1);
                 INSERT INTO calendars(id,account_id,remote_id,name,color_hex,can_edit) VALUES('ro','a','RRO','Holidays','#000',0);
                 INSERT INTO calendar_events(id,calendar_id,remote_id,ical_uid,event_type,title,start_utc,end_utc,original_timezone,attendees_json,my_response_status)
                   VALUES('ok','c','r1','u1','x','Team meeting',1,2,'UTC','[{\"email\":\"b@x.com\"}]','notResponded'),
                         ('mine','c','r2','u2','x','My meeting',1,2,'UTC','[{\"email\":\"b@x.com\"}]','organizer'),
                         ('solo','c','r3','u3','x','Dentist',1,2,'UTC','[]','none'),
                         ('hol','ro','r4','u4','x','Public holiday',1,2,'UTC','[{\"email\":\"b@x.com\"}]','none');",
            )?;
            Ok(())
        })
        .unwrap();
        for id in ["mine", "solo", "hol"] {
            assert!(rsvp(id, "accept").is_err(), "{id} must be refused");
        }
        assert!(rsvp("ok", "bogus").is_err());
        let queued = |id: &str| -> i64 {
            storage::with_db(|db| Ok(db.query_row("SELECT COUNT(*) FROM offline_mutation_queue WHERE local_entity_id=?1", [id], |r| r.get(0))?)).unwrap()
        };
        assert_eq!((queued("mine"), queued("solo"), queued("hol")), (0, 0, 0));
        rsvp("ok", "tentativelyAccept").unwrap();
        rsvp("ok", "decline").unwrap(); // replaces the earlier queued answer
        assert_eq!(queued("ok"), 1);
        let status: String = storage::with_db(|db| Ok(db.query_row("SELECT my_response_status FROM calendar_events WHERE id='ok'", [], |r| r.get(0))?)).unwrap();
        assert_eq!(status, "declined");
    }
}
