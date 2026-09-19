use rusqlite::{params, Connection};

pub fn open(path: &str) -> rusqlite::Result<Connection> {
    let c = Connection::open(path)?;
    c.execute_batch(
        "pragma journal_mode=wal;
         pragma foreign_keys=on;
         create table if not exists records(
           id integer primary key,
           user_id integer not null,
           mod_id integer not null,
           kind text not null,
           reason text not null default '',
           created integer not null,
           expires integer,
           undone integer not null default 0
         );
         create index if not exists records_user on records(user_id);
         create index if not exists records_due on records(kind, undone, expires);
         create table if not exists kv(k text primary key, v text not null);
         create table if not exists msgcache(
           id integer primary key,
           channel_id integer not null,
           author_id integer not null,
           content text not null,
           files text not null default '',
           created integer not null
         );
         create index if not exists msgcache_age on msgcache(created);",
    )?;
    Ok(c)
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug)]
pub struct Record {
    pub id: i64,
    pub user_id: u64,
    pub mod_id: u64,
    pub kind: String,
    pub reason: String,
    pub created: i64,
    pub expires: Option<i64>,
    pub undone: bool,
}

pub fn add(
    c: &Connection,
    user: u64,
    moderator: u64,
    kind: &str,
    reason: &str,
    expires: Option<i64>,
) -> rusqlite::Result<i64> {
    c.execute(
        "insert into records(user_id, mod_id, kind, reason, created, expires)
         values(?1, ?2, ?3, ?4, ?5, ?6)",
        params![user as i64, moderator as i64, kind, reason, now(), expires],
    )?;
    Ok(c.last_insert_rowid())
}

pub fn list(c: &Connection, user: u64, limit: i64) -> rusqlite::Result<Vec<Record>> {
    let mut st = c.prepare(&format!(
        "{SELECT} where user_id = ?1 order by created desc limit ?2"
    ))?;
    let rows = st.query_map(params![user as i64, limit], row_to_record)?;
    rows.collect()
}

const SELECT: &str =
    "select id, user_id, mod_id, kind, reason, created, expires, undone from records";

fn row_to_record(r: &rusqlite::Row) -> rusqlite::Result<Record> {
    Ok(Record {
        id: r.get(0)?,
        user_id: r.get::<_, i64>(1)? as u64,
        mod_id: r.get::<_, i64>(2)? as u64,
        kind: r.get(3)?,
        reason: r.get(4)?,
        created: r.get(5)?,
        expires: r.get(6)?,
        undone: r.get::<_, i64>(7)? != 0,
    })
}

pub fn get(c: &Connection, id: i64) -> rusqlite::Result<Option<Record>> {
    c.query_row(&format!("{SELECT} where id = ?1"), [id], row_to_record)
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            e => Err(e),
        })
}

pub fn set_reason(c: &Connection, id: i64, reason: &str) -> rusqlite::Result<usize> {
    c.execute("update records set reason = ?2 where id = ?1", params![id, reason])
}

pub fn count(c: &Connection, user: u64) -> rusqlite::Result<i64> {
    c.query_row(
        "select count(*) from records where user_id = ?1",
        [user as i64],
        |r| r.get(0),
    )
}

pub fn delete(c: &Connection, id: i64) -> rusqlite::Result<usize> {
    c.execute("delete from records where id = ?1", [id])
}

pub fn clear(c: &Connection, user: u64) -> rusqlite::Result<usize> {
    c.execute("delete from records where user_id = ?1", [user as i64])
}

pub fn due_unbans(c: &Connection) -> rusqlite::Result<Vec<(i64, u64)>> {
    let mut st = c.prepare(
        "select id, user_id from records
         where kind = 'ban' and undone = 0 and expires is not null and expires <= ?1",
    )?;
    let rows = st.query_map([now()], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? as u64)))?;
    rows.collect()
}

pub fn mark_undone(c: &Connection, id: i64) -> rusqlite::Result<usize> {
    c.execute("update records set undone = 1 where id = ?1", [id])
}

pub fn mark_bans_undone(c: &Connection, user: u64) -> rusqlite::Result<usize> {
    c.execute(
        "update records set undone = 1 where user_id = ?1 and kind = 'ban' and undone = 0",
        [user as i64],
    )
}

pub fn kv_get(c: &Connection, k: &str) -> rusqlite::Result<Option<String>> {
    c.query_row("select v from kv where k = ?1", [k], |r| r.get(0))
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            e => Err(e),
        })
}

pub fn kv_set(c: &Connection, k: &str, v: &str) -> rusqlite::Result<usize> {
    c.execute(
        "insert into kv(k, v) values(?1, ?2) on conflict(k) do update set v = excluded.v",
        params![k, v],
    )
}

pub fn kv_u64(c: &Connection, k: &str) -> Option<u64> {
    kv_get(c, k).ok().flatten().and_then(|v| v.parse().ok())
}

pub struct Cached {
    pub author_id: u64,
    pub content: String,
    pub files: String,
}

pub fn cache_message(
    c: &Connection,
    id: u64,
    channel: u64,
    author: u64,
    content: &str,
    files: &str,
) -> rusqlite::Result<usize> {
    c.execute(
        "insert or replace into msgcache(id, channel_id, author_id, content, files, created)
         values(?1, ?2, ?3, ?4, ?5, ?6)",
        params![id as i64, channel as i64, author as i64, content, files, now()],
    )
}

pub fn cached_message(c: &Connection, id: u64) -> rusqlite::Result<Option<Cached>> {
    c.query_row(
        "select author_id, content, files from msgcache where id = ?1",
        [id as i64],
        |r| {
            Ok(Cached {
                author_id: r.get::<_, i64>(0)? as u64,
                content: r.get(1)?,
                files: r.get(2)?,
            })
        },
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        e => Err(e),
    })
}

pub fn prune_cache(c: &Connection, keep_days: i64) -> rusqlite::Result<usize> {
    c.execute(
        "delete from msgcache where created < ?1",
        [now() - keep_days * 86_400],
    )
}

pub fn next_unban(c: &Connection) -> rusqlite::Result<Option<i64>> {
    c.query_row(
        "select min(expires) from records
         where kind = 'ban' and undone = 0 and expires is not null",
        [],
        |r| r.get::<_, Option<i64>>(0),
    )
}

pub fn browse(
    c: &Connection,
    kind: Option<String>,
    mod_id: Option<u64>,
    search: Option<String>,
    limit: i64,
) -> rusqlite::Result<Vec<Record>> {
    let mut sql = format!("{SELECT} where 1 = 1");
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(k) = kind {
        sql.push_str(" and kind = ?");
        args.push(Box::new(k));
    }
    if let Some(m) = mod_id {
        sql.push_str(" and mod_id = ?");
        args.push(Box::new(m as i64));
    }
    if let Some(q) = search {
        sql.push_str(" and reason like ? escape '\\'");
        let escaped = q
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        args.push(Box::new(format!("%{escaped}%")));
    }
    sql.push_str(" order by created desc limit ?");
    args.push(Box::new(limit));

    let mut st = c.prepare(&sql)?;
    let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
    let rows = st.query_map(rusqlite::params_from_iter(refs), row_to_record)?;
    rows.collect()
}
