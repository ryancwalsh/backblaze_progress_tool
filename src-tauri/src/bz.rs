//! Reads Backblaze Personal Backup's local data files (read-only) and works out,
//! for every file in the current to-do list, its status and the most likely reason
//! it has not been uploaded yet. Nothing here writes to Backblaze's folders and
//! nothing is sent over the network.
//!
//! File formats were worked out from the Windows client 9.2.x; see README.md.

use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Output types (serialised to the UI as camelCase JSON)
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub path: String,
    pub size: u64,
    /// File modification time recorded when Backblaze queued it (ms since epoch).
    pub queued_mtime: u64,
    pub exists: Option<bool>,
    pub cur_size: Option<u64>,
    pub cur_mtime: Option<u64>,
    pub ext: String,
    pub drive: String,
    pub folder: String,
    pub name: String,
    /// Short machine code, e.g. "uploaded", "busy", "missing", "waiting".
    pub status: String,
    /// Human explanation.
    pub reason: String,
    pub skip_code: Option<String>,
    pub skip_time: Option<String>,
    pub log_time: Option<String>,
    pub log: Option<String>,
    pub uploaded_at: Option<u64>,
    pub chunks_sent: u32,
    pub chunk_bytes_sent: u64,
    pub open_error: Option<String>,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Volume {
    pub guid: String,
    pub drive: String,
    pub bytes_free: Option<u64>,
    pub bytes_total: Option<u64>,
    pub remaining_files: Option<u64>,
    pub remaining_bytes: Option<u64>,
    pub selected_files: Option<u64>,
    pub selected_bytes: Option<u64>,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Current {
    pub state: Option<String>,
    pub file: Option<String>,
    pub full_path: Option<String>,
    pub written_ms: Option<u64>,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LargeFile {
    pub path: String,
    pub bytes: Option<u64>,
    pub started_ms: Option<u64>,
    pub chunks_sent: u32,
    pub chunk_bytes_sent: u64,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub client_version: Option<String>,
    pub threads: Option<String>,
    pub throttle: Option<String>,
    pub auto_throttle: Option<String>,
    pub schedule: Option<String>,
    pub max_filesize_mb: Option<String>,
    pub measured_kbps_large: Option<u64>,
    pub measured_kbps_small: Option<u64>,
    pub last_kbps: Option<u64>,
    pub safety_frozen: Option<String>,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Exclusions {
    pub file_types: Vec<String>,
    pub dirs: Vec<String>,
    pub editable_rules: Vec<HashMap<String, String>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthItem {
    pub level: String, // "warn" | "info"
    pub title: String,
    pub detail: String,
    pub count: u64,
    pub first: Option<String>,
    pub last: Option<String>,
    pub examples: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub time: String,
    pub text: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Skipped {
    pub time: String,
    pub code: String,
    pub path: String,
    pub in_todo: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct HistPoint {
    pub t: u64,
    pub files: u64,
    pub bytes: u64,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub generated_ms: u64,
    pub data_dir: String,
    pub todo_file: String,
    pub todo_written_ms: u64,
    pub rows: Vec<Row>,
    pub remaining_files: Option<u64>,
    pub remaining_bytes: Option<u64>,
    pub remaining_written_ms: Option<u64>,
    pub selected_files: Option<u64>,
    pub selected_bytes: Option<u64>,
    pub last_backup_completed_ms: Option<u64>,
    pub volumes: Vec<Volume>,
    pub current: Current,
    pub large_file: Option<LargeFile>,
    pub settings: Settings,
    pub exclusions: Exclusions,
    pub health: Vec<HealthItem>,
    pub events: Vec<Event>,
    pub skipped: Vec<Skipped>,
    pub history: Vec<HistPoint>,
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

pub fn default_data_dir() -> PathBuf {
    if let Ok(p) = std::env::var("BZDATA") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    platform_data_dir()
}

#[cfg(windows)]
fn platform_data_dir() -> PathBuf {
    let pd = std::env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".to_string());
    PathBuf::from(pd).join("Backblaze").join("bzdata")
}

#[cfg(not(windows))]
fn platform_data_dir() -> PathBuf {
    // macOS location of the Backblaze client's data
    PathBuf::from("/Library/Backblaze.bzpkg/bzdata")
}

fn sys_ms(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn now_ms() -> u64 {
    sys_ms(SystemTime::now())
}

fn mtime_ms(p: &Path) -> Option<u64> {
    fs::metadata(p).ok()?.modified().ok().map(sys_ms)
}

fn hex_u64(s: &str) -> Option<u64> {
    u64::from_str_radix(s.trim(), 16).ok()
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// "20261113212429" (GMT) -> ms since epoch
fn gmt14_to_ms(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.len() != 14 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n = |a: usize, b: usize| s[a..b].parse::<i64>().ok();
    let (y, mo, d, h, mi, se) = (n(0, 4)?, n(4, 6)?, n(6, 8)?, n(8, 10)?, n(10, 12)?, n(12, 14)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    let days = days_from_civil(y, mo, d);
    let secs = days * 86400 + h * 3600 + mi * 60 + se;
    if secs < 0 {
        None
    } else {
        Some(secs as u64 * 1000)
    }
}

/// Calls `f` for each line of a file (lossy UTF-8, line endings stripped).
fn for_each_line(p: &Path, mut f: impl FnMut(&str)) -> io::Result<()> {
    let file = File::open(p)?;
    let mut r = BufReader::with_capacity(1 << 20, file);
    let mut buf = Vec::with_capacity(4096);
    let mut first = true;
    loop {
        buf.clear();
        let n = r.read_until(b'\n', &mut buf)?;
        if n == 0 {
            break;
        }
        while matches!(buf.last(), Some(b'\n') | Some(b'\r')) {
            buf.pop();
        }
        let mut s = String::from_utf8_lossy(&buf);
        if first {
            first = false;
            if let Some(stripped) = s.strip_prefix('\u{feff}') {
                s = std::borrow::Cow::Owned(stripped.to_string());
            }
        }
        f(&s);
    }
    Ok(())
}

fn read_text(p: &Path) -> Option<String> {
    let bytes = fs::read(p).ok()?;
    let s = String::from_utf8_lossy(&bytes).into_owned();
    Some(s.trim_start_matches('\u{feff}').to_string())
}

fn xml_unescape(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn strip_xml_comments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("<!--") {
        out.push_str(&rest[..i]);
        match rest[i..].find("-->") {
            Some(j) => rest = &rest[i + j + 3..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

fn parse_attrs(s: &str) -> HashMap<String, String> {
    let mut m = HashMap::new();
    let b = s.as_bytes();
    let mut i = 0;
    while let Some(off) = s[i..].find("=\"") {
        let eq = i + off;
        let mut k = eq;
        while k > 0 && (b[k - 1].is_ascii_alphanumeric() || matches!(b[k - 1], b'_' | b'-' | b':')) {
            k -= 1;
        }
        let vstart = eq + 2;
        let vend = match s[vstart..].find('"') {
            Some(e) => vstart + e,
            None => break,
        };
        if k < eq {
            m.insert(s[k..eq].to_string(), xml_unescape(&s[vstart..vend]));
        }
        i = vend + 1;
    }
    m
}

/// All `<tag ...>` elements' attributes, in document order.
fn xml_elements(text: &str, tag: &str) -> Vec<HashMap<String, String>> {
    let pat = format!("<{}", tag);
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(off) = text[i..].find(&pat) {
        let start = i + off + pat.len();
        let next = text[start..].chars().next();
        if !matches!(next, Some(c) if c.is_whitespace() || c == '/' || c == '>') {
            i = start;
            continue;
        }
        let end = text[start..].find('>').map(|e| start + e).unwrap_or(text.len());
        out.push(parse_attrs(&text[start..end]));
        i = end;
    }
    out
}

fn xml_first(text: &str, tag: &str) -> HashMap<String, String> {
    xml_elements(text, tag).into_iter().next().unwrap_or_default()
}

fn num(m: &HashMap<String, String>, k: &str) -> Option<u64> {
    m.get(k).and_then(|v| v.trim().parse::<u64>().ok())
}

/// Finds the first Windows (`X:\`, `\\server`) or POSIX absolute path in a log line.
fn find_path(s: &str) -> Option<&str> {
    let b = s.as_bytes();
    for i in 0..b.len().saturating_sub(2) {
        let prev_ok = i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_');
        if prev_ok && b[i].is_ascii_alphabetic() && b[i + 1] == b':' && (b[i + 2] == b'\\' || b[i + 2] == b'/') {
            return Some(s[i..].trim_end().trim_end_matches('"'));
        }
        if i > 0 && b[i] == b'\\' && b[i + 1] == b'\\' && b[i - 1] == b' ' {
            return Some(s[i..].trim_end());
        }
    }
    // POSIX paths (macOS) appear after ": " or "= " in log lines
    for pat in [": /", "=/", "= /", "fileName=/"] {
        if let Some(j) = s.find(pat) {
            let start = j + pat.len() - 1;
            return Some(s[start..].trim_end());
        }
    }
    None
}

fn key(p: &str) -> String {
    p.trim().to_lowercase()
}

/// Splits "2026-11-13 00:00:26       3592 - message" into (time, message).
fn split_log_line(l: &str) -> (String, String) {
    let ts = if l.len() >= 19 && l.is_char_boundary(19) { l[..19].to_string() } else { String::new() };
    let msg = match l.find(" - ") {
        Some(i) => l[i + 3..].trim().to_string(),
        None => l.to_string(),
    };
    (ts, msg)
}

fn path_parts(p: &str) -> (String, String, String, String) {
    let sep = |c: char| c == '\\' || c == '/';
    let name = p.rsplit(sep).next().unwrap_or(p).to_string();
    let ext = match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => name[i + 1..].to_lowercase(),
        _ => String::new(),
    };
    let drive = if p.len() >= 2 && p.as_bytes()[1] == b':' {
        p[..2].to_uppercase()
    } else if p.starts_with("\\\\") {
        "UNC".to_string()
    } else {
        "/".to_string()
    };
    // first three path components, e.g. C:\Users\name
    let comps: Vec<&str> = p.split(sep).filter(|c| !c.is_empty()).collect();
    let n = comps.len().saturating_sub(1).min(3); // exclude the file name
    let mut folder = comps[..n].join("\\");
    if drive == "/" {
        folder = format!("/{}", comps[..n].join("/"));
    }
    (name, ext, drive, folder)
}

fn list_dir(p: &Path) -> Vec<(PathBuf, String, u64)> {
    let mut v = Vec::new();
    if let Ok(rd) = fs::read_dir(p) {
        for e in rd.flatten() {
            let path = e.path();
            if path.is_file() {
                let name = e.file_name().to_string_lossy().into_owned();
                let m = e.metadata().ok().and_then(|m| m.modified().ok()).map(sys_ms).unwrap_or(0);
                v.push((path, name, m));
            }
        }
    }
    v
}

fn fmt_count(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

// ---------------------------------------------------------------------------
// Main entry
// ---------------------------------------------------------------------------

struct Counter {
    count: u64,
    first: Option<String>,
    last: Option<String>,
    examples: Vec<String>,
    distinct: HashSet<String>,
}

impl Counter {
    fn new() -> Self {
        Counter { count: 0, first: None, last: None, examples: vec![], distinct: HashSet::new() }
    }
    fn hit(&mut self, ts: &str) {
        self.count += 1;
        if self.first.is_none() {
            self.first = Some(ts.to_string());
        }
        self.last = Some(ts.to_string());
    }
}

pub fn load(data_dir: &Path, check_files: bool, history_dir: Option<PathBuf>) -> Result<Snapshot, String> {
    let mut snap = Snapshot { generated_ms: now_ms(), data_dir: data_dir.display().to_string(), ..Default::default() };

    let dc = data_dir.join("bzbackup").join("bzdatacenter");
    let reports = data_dir.join("bzreports");
    let logs = data_dir.join("bzlogs");

    match fs::read_dir(&dc) {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            return Err(format!(
                "Permission denied reading {}. Run this app as Administrator (Windows) or with full disk access / sudo (Mac).",
                dc.display()
            ))
        }
        Err(e) => return Err(format!("Cannot read Backblaze data folder {}: {}", dc.display(), e)),
    }

    // ---- 1. newest to-do list -------------------------------------------------
    let todo = list_dir(&dc)
        .into_iter()
        .filter(|(_, n, _)| n.starts_with("bz_todo_") && n.ends_with(".dat"))
        .max_by_key(|(_, _, m)| *m);
    let mut rows: Vec<Row> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    if let Some((tp, tn, tm)) = &todo {
        snap.todo_file = tn.clone();
        snap.todo_written_ms = *tm;
        for_each_line(tp, |l| {
            let f: Vec<&str> = l.splitn(6, '\t').collect();
            if f.len() < 6 {
                return;
            }
            let path = f[5].to_string();
            if path.is_empty() {
                return;
            }
            let (name, ext, drive, folder) = path_parts(&path);
            let row = Row {
                size: f[4].trim().parse().unwrap_or(0),
                queued_mtime: hex_u64(f[3]).unwrap_or(0),
                name,
                ext,
                drive,
                folder,
                path: path.clone(),
                ..Default::default()
            };
            index.insert(key(&path), rows.len());
            rows.push(row);
        })
        .map_err(|e| format!("Reading {}: {}", tn, e))?;
    } else {
        snap.warnings.push("No bz_todo_*.dat file found: Backblaze may be fully backed up, or between scans.".into());
    }
    let todo_ms = snap.todo_written_ms;

    // ---- 2. uploads recorded since the to-do list was written -------------------
    for (p, n, m) in list_dir(&dc) {
        if !(n.starts_with("bz_done_") && n.ends_with(".dat")) || m + 60_000 < todo_ms {
            continue;
        }
        let _ = for_each_line(&p, |l| {
            let f: Vec<&str> = l.split('\t').collect();
            if f.len() < 14 {
                return;
            }
            let t = match gmt14_to_ms(f[3]) {
                Some(t) => t,
                None => return,
            };
            if t < todo_ms {
                return;
            }
            let path = f[13..].join("\t");
            if let Some(&i) = index.get(&key(&path)) {
                let r = &mut rows[i];
                let is_chunk = !f[7].starts_with("k0_") && f[1] != "!";
                if f[1] == "!" {
                    // large-file completion record
                    r.uploaded_at = Some(t);
                } else if is_chunk {
                    r.chunks_sent += 1;
                    r.chunk_bytes_sent += f[12].trim().parse::<u64>().unwrap_or(0);
                } else if f[1] == "+" || f[1] == "=" {
                    r.uploaded_at = Some(t);
                }
            }
        });
    }

    // ---- 3. skipped-files report (closest thing to the "Issues Report") ----------
    let mut skip_map: HashMap<String, (String, String)> = HashMap::new();
    for name in ["bzlist_skipped_files.txt.old", "bzlist_skipped_files.txt"] {
        let _ = for_each_line(&reports.join(name), |l| {
            if l.starts_with('#') || l.trim().is_empty() {
                return;
            }
            let f: Vec<&str> = l.split('\t').map(|x| x.trim()).collect();
            if f.len() < 3 {
                return;
            }
            let path = f.iter().rev().find(|x| !x.is_empty()).map(|x| x.to_string()).unwrap_or_default();
            let k = key(&path);
            let rec = Skipped { time: f[0].to_string(), code: f[1].to_string(), path: path.clone(), in_todo: index.contains_key(&k) };
            skip_map.insert(k, (rec.code.clone(), rec.time.clone()));
            snap.skipped.retain(|s| key(&s.path) != key(&rec.path));
            snap.skipped.push(rec);
        });
    }

    // ---- 4. transmit / filelist logs ---------------------------------------------
    let mut bad_hguid = Counter::new();
    let mut self_heal = Counter::new();
    let mut gone = Counter::new();
    let mut scan_fail = Counter::new();
    let mut log_map: HashMap<String, (String, String)> = HashMap::new();

    let mut tx_logs: Vec<(PathBuf, String, u64)> = list_dir(&logs.join("bztransmit"))
        .into_iter()
        .filter(|(_, n, _)| n.ends_with(".log"))
        .collect();
    tx_logs.sort_by_key(|x| x.2);
    for (p, _, _) in &tx_logs {
        let _ = for_each_line(p, |l| {
            if l.contains("bad hguid bz_done line") {
                let (ts, _) = split_log_line(l);
                bad_hguid.hit(&ts);
                return;
            }
            if l.contains("during self healing datacenter told me") {
                let (ts, _) = split_log_line(l);
                self_heal.hit(&ts);
                if let Some(i) = l.find("bzfileid_short=") {
                    let id: String = l[i + 15..].chars().take_while(|c| c.is_ascii_hexdigit()).collect();
                    self_heal.distinct.insert(id);
                }
                return;
            }
            if l.contains("fourHourLock") {
                return;
            }
            let interesting = l.contains("WARNING") || l.contains("ERROR") || l.contains("skipping") || l.contains("FAIL");
            if !interesting {
                return;
            }
            if let Some(p) = find_path(l) {
                let k = key(p);
                let (ts, msg) = split_log_line(l);
                if l.contains("no longer exists") {
                    gone.hit(&ts);
                }
                if index.contains_key(&k) {
                    log_map.insert(k, (ts, msg));
                }
            }
        });
    }
    let mut fl_logs: Vec<(PathBuf, String, u64)> = list_dir(&logs.join("bzfilelist"))
        .into_iter()
        .filter(|(_, n, _)| n.ends_with(".log"))
        .collect();
    fl_logs.sort_by_key(|x| x.2);
    for (p, _, _) in &fl_logs {
        let _ = for_each_line(p, |l| {
            if l.contains("FindFirstFile") && l.contains("failed") {
                let (ts, _) = split_log_line(l);
                scan_fail.hit(&ts);
                if let Some(i) = l.find("dirPat=") {
                    let d = l[i + 7..].trim().to_string();
                    if scan_fail.distinct.insert(d.clone()) && scan_fail.examples.len() < 15 {
                        scan_fail.examples.push(d);
                    }
                }
            }
        });
    }

    // ---- 5. status / stats XML files ----------------------------------------------
    // current transmit status: an XML in bzdata root containing <bztransmit cur_state=...>
    for (p, _, m) in list_dir(data_dir) {
        if p.extension().map(|e| e == "xml").unwrap_or(false) {
            if let Some(t) = read_text(&p) {
                if t.contains("<bztransmit") && t.contains("cur_state") {
                    let a = xml_first(&t, "bztransmit");
                    snap.current = Current {
                        state: a.get("cur_state").cloned(),
                        file: a.get("current_file").cloned(),
                        full_path: a.get("current_file_fullpath").cloned(),
                        written_ms: Some(m),
                    };
                }
            }
        }
    }
    // large file in progress
    for (p, _, _) in list_dir(&dc.join("bzcurrentlargefile")) {
        if let Some(t) = read_text(&p) {
            if t.contains("bzonelargefile") {
                let one = xml_first(&t, "bzonelargefile");
                let info = xml_first(&t, "largefileinfo");
                let other = xml_first(&t, "otherinfo");
                if let Some(path) = one.get("bzfname") {
                    let mut lf = LargeFile {
                        path: path.clone(),
                        bytes: num(&info, "numbytesinfile"),
                        started_ms: num(&other, "millis_gmt_backup_started"),
                        ..Default::default()
                    };
                    if let Some(&i) = index.get(&key(path)) {
                        lf.chunks_sent = rows[i].chunks_sent;
                        lf.chunk_bytes_sent = rows[i].chunk_bytes_sent;
                    }
                    snap.large_file = Some(lf);
                }
            }
        }
    }

    // volumes
    let mut vols: Vec<Volume> = Vec::new();
    if let Some(t) = read_text(&data_dir.join("bzvolumes.xml")) {
        for a in xml_elements(&t, "bzvolume") {
            let guid = a.get("bzVolumeGuid").cloned().unwrap_or_default();
            let drive = a
                .get("mountPointPathHex")
                .and_then(|h| {
                    let bytes: Option<Vec<u8>> =
                        (0..h.len() / 2).map(|i| u8::from_str_radix(&h[i * 2..i * 2 + 2], 16).ok()).collect();
                    bytes.map(|b| String::from_utf8_lossy(&b).into_owned())
                })
                .unwrap_or_default();
            vols.push(Volume { guid, drive, bytes_free: num(&a, "numBytesFreeOnVolume"), bytes_total: num(&a, "numBytesTotalOnVolume"), ..Default::default() });
        }
    }
    let vol_mut = |vols: &mut Vec<Volume>, guid: &str| -> usize {
        if let Some(i) = vols.iter().position(|v| v.guid == guid) {
            i
        } else {
            vols.push(Volume { guid: guid.to_string(), ..Default::default() });
            vols.len() - 1
        }
    };
    let rem_path = reports.join("bzstat_remainingbackup.xml");
    if let Some(t) = read_text(&rem_path) {
        let r = xml_first(&t, "remaining");
        snap.remaining_files = num(&r, "remainingnumfilesforbackup");
        snap.remaining_bytes = num(&r, "remainingnumbytesforbackup");
        snap.remaining_written_ms = mtime_ms(&rem_path);
        for a in xml_elements(&t, "bzvolume") {
            if let Some(g) = a.get("bzVolumeGuid") {
                let i = vol_mut(&mut vols, g);
                vols[i].remaining_files = num(&a, "pervol_remaining_files_numfiles");
                vols[i].remaining_bytes = num(&a, "pervol_remaining_files_numbytes");
            }
        }
    }
    if let Some(t) = read_text(&reports.join("bzstat_totalbackup.xml")) {
        let r = xml_first(&t, "totals");
        snap.selected_files = num(&r, "totnumfilesforbackup");
        snap.selected_bytes = num(&r, "totnumbytesforbackup");
        for a in xml_elements(&t, "bzvolume") {
            if let Some(g) = a.get("bzVolumeGuid") {
                let i = vol_mut(&mut vols, g);
                vols[i].selected_files = num(&a, "pervol_sel_for_backup_numfiles");
                vols[i].selected_bytes = num(&a, "pervol_sel_for_backup_numbytes");
            }
        }
    }
    snap.volumes = vols;
    if let Some(t) = read_text(&reports.join("bzstat_lastbackupcompleted.xml")) {
        snap.last_backup_completed_ms = num(&xml_first(&t, "lastbackupcompleted"), "gmt_millis");
    }

    // settings
    let mut st = Settings::default();
    st.client_version = read_text(&reports.join("bzserv_version.txt")).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let mut excl = Exclusions::default();
    if let Some(t) = read_text(&data_dir.join("bzinfo.xml")) {
        let t = strip_xml_comments(&t);
        let d = xml_first(&t, "do_backup");
        st.threads = d.get("num_backup_threads").cloned();
        st.throttle = d.get("net_throttle").cloned();
        st.auto_throttle = d.get("net_auto_throttle").cloned();
        st.schedule = d.get("backup_schedule_type").cloned();
        st.max_filesize_mb = d.get("max_filesize_mb").cloned();
        if let Some(ft) = xml_first(&t, "globalexcludes").get("excludefiletypes") {
            excl.file_types = ft.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        }
        for a in xml_elements(&t, "bzdirfilter") {
            if a.get("whichfiles").map(|w| w == "none").unwrap_or(false) {
                if let Some(d) = a.get("dir") {
                    excl.dirs.push(d.clone());
                }
            }
        }
    }
    if let Some(t) = read_text(&data_dir.join("bzexcluderules_editable.xml")) {
        excl.editable_rules = xml_elements(&strip_xml_comments(&t), "excludefname_rule");
    }
    snap.exclusions = excl;
    if let Some(t) = read_text(&reports.join("bzperf_measured_upload.xml")) {
        let m = xml_first(&t, "measured");
        st.measured_kbps_large = num(&m, "perf_in_kbits_per_sec_larger_1mb");
        st.measured_kbps_small = num(&m, "perf_in_kbits_per_sec_smaller_1mb");
    }
    if let Some(t) = read_text(&logs.join("bzreports_lastfilestransmitted").join("bzstat_lastfile_transmitted.xml")) {
        st.last_kbps = num(&xml_first(&t, "lastfile_transmitted"), "kBitsPerSec_of_lastActualTransmission");
    }
    if let Some(t) = read_text(&reports.join("bzdc_filecheck.xml")) {
        st.safety_frozen = xml_first(&t, "metainfo").get("safety_frozen").cloned();
    }

    // backup pass history (event log)
    let mut ev_files: Vec<(PathBuf, String, u64)> = list_dir(&logs.join("bzreports_eventlog"))
        .into_iter()
        .filter(|(_, n, _)| n.ends_with(".log"))
        .collect();
    ev_files.sort_by_key(|x| x.2);
    let start = ev_files.len().saturating_sub(3);
    for (p, _, _) in &ev_files[start..] {
        let _ = for_each_line(p, |l| {
            if let Some(i) = l.find(" - ") {
                snap.events.push(Event { time: l[..i].trim().to_string(), text: l[i + 3..].trim().to_string() });
            }
        });
    }
    let ev_len = snap.events.len();
    if ev_len > 80 {
        snap.events.drain(..ev_len - 80);
    }
    snap.events.reverse();

    // ---- 6. per-file status ------------------------------------------------------
    let large_key = snap.large_file.as_ref().map(|l| key(&l.path));
    let current_key = snap.current.full_path.as_ref().map(|p| key(p));
    for r in rows.iter_mut() {
        let k = key(&r.path);
        if let Some((code, time)) = skip_map.get(&k) {
            r.skip_code = Some(code.clone());
            r.skip_time = Some(time.clone());
        }
        if let Some((ts, msg)) = log_map.get(&k) {
            r.log_time = Some(ts.clone());
            r.log = Some(msg.clone());
        }
        let done = r.uploaded_at.is_some();
        if check_files && !done {
            check_on_disk(r);
        }
        let (status, reason) = classify(r, large_key.as_deref() == Some(k.as_str()) || current_key.as_deref() == Some(k.as_str()));
        r.status = status.to_string();
        r.reason = reason;
    }

    // ---- 7. health summary ------------------------------------------------------
    let mut health = Vec::new();
    if let Some(sf) = &st.safety_frozen {
        if sf != "not_frozen" {
            health.push(HealthItem { level: "warn".into(), title: "Backup is safety-frozen".into(), detail: format!("Backblaze reports safety_frozen=\"{}\". Contact Backblaze support.", sf), count: 1, first: None, last: None, examples: vec![] });
        }
    }
    if bad_hguid.count > 0 {
        health.push(HealthItem {
            level: "warn".into(),
            title: "Unreadable lines in Backblaze's upload history".into(),
            detail: format!("bztransmit logged {} \"bad hguid bz_done line, skipped\" errors while reading its own record of uploaded files. Files whose records it can't read may be queued again. Worth reporting to Backblaze support.", fmt_count(bad_hguid.count)),
            count: bad_hguid.count,
            first: bad_hguid.first.clone(),
            last: bad_hguid.last.clone(),
            examples: vec![],
        });
    }
    if self_heal.count > 0 {
        let mut ids: Vec<String> = self_heal.distinct.iter().cloned().collect();
        ids.sort();
        health.push(HealthItem {
            level: if self_heal.count > 50 { "warn" } else { "info" }.into(),
            title: "Repeating self-healing purge request".into(),
            detail: format!("The data center asked {} time(s) to purge {} file ID(s) from the local upload history. If this keeps repeating, the self-repair may be stuck.", fmt_count(self_heal.count), ids.len()),
            count: self_heal.count,
            first: self_heal.first.clone(),
            last: self_heal.last.clone(),
            examples: ids.into_iter().take(10).collect(),
        });
    }
    if let Some(t) = read_text(&reports.join("bzlist_bzdone_retransmit.txt")) {
        let lines: Vec<&str> = t.lines().filter(|l| !l.trim().is_empty()).collect();
        if !lines.is_empty() {
            let last = lines.last().and_then(|l| l.split_whitespace().next()).map(|s| s.to_string());
            health.push(HealthItem {
                level: "info".into(),
                title: "Upload-history files Backblaze couldn't read".into(),
                detail: format!("bzlist_bzdone_retransmit.txt has {} entries (for example \"BzReadFile_failed\").", lines.len()),
                count: lines.len() as u64,
                first: lines.first().and_then(|l| l.split_whitespace().next()).map(|s| s.to_string()),
                last,
                examples: lines.iter().take(5).map(|s| s.to_string()).collect(),
            });
        }
    }
    if gone.count > 0 {
        health.push(HealthItem { level: "info".into(), title: "Files deleted before they could upload".into(), detail: "\"skipping file because it no longer exists\" in the transmit log. Normal for temp/cache files, but a lot of these means something busy is in your backup set.".into(), count: gone.count, first: gone.first.clone(), last: gone.last.clone(), examples: vec![] });
    }
    if scan_fail.count > 0 {
        health.push(HealthItem { level: "info".into(), title: "Folders Backblaze couldn't scan".into(), detail: "bzfilelist could not list these folders, so files inside them are not being backed up.".into(), count: scan_fail.count, first: scan_fail.first.clone(), last: scan_fail.last.clone(), examples: scan_fail.examples.clone() });
    }
    let busy = snap.skipped.iter().filter(|s| s.code.contains("BUSY")).count() as u64;
    let perm = snap.skipped.iter().filter(|s| s.code.contains("PERMISSION")).count() as u64;
    if perm > 0 {
        health.push(HealthItem { level: "warn".into(), title: "Files Backblaze has no permission to read".into(), detail: "PERMANENT_BAD_PERMISSIONS in the skipped-files report: these won't be retried until permissions change or you exclude them.".into(), count: perm, first: None, last: None, examples: snap.skipped.iter().filter(|s| s.code.contains("PERMISSION")).map(|s| s.path.clone()).take(10).collect() });
    }
    if busy > 0 {
        health.push(HealthItem { level: "info".into(), title: "Files in use when Backblaze tried".into(), detail: "TEMPORARY_FILE_BUSY in the skipped-files report: retried automatically. If the same files show up every time, close the program holding them or exclude them.".into(), count: busy, first: None, last: None, examples: vec![] });
    }
    if st.threads.as_deref() == Some("1") {
        health.push(HealthItem { level: "info".into(), title: "Uploading with a single thread".into(), detail: "Many small files upload much faster with more threads (Backblaze Settings > Performance).".into(), count: 1, first: None, last: None, examples: vec![] });
    }
    snap.health = health;
    snap.settings = st;
    snap.rows = rows;

    // ---- 8. remaining-backup history kept by this app --------------------------
    if let (Some(dir), Some(files), Some(bytes)) = (history_dir, snap.remaining_files, snap.remaining_bytes) {
        snap.history = update_history(&dir, files, bytes);
    }
    Ok(snap)
}

fn check_on_disk(r: &mut Row) {
    match fs::metadata(&r.path) {
        Ok(md) => {
            r.exists = Some(true);
            r.cur_size = Some(md.len());
            r.cur_mtime = md.modified().ok().map(sys_ms);
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                let a = md.file_attributes();
                // OFFLINE | RECALL_ON_OPEN | RECALL_ON_DATA_ACCESS
                if a & (0x1000 | 0x40000 | 0x400000) != 0 {
                    r.open_error = Some("cloud-only placeholder".into());
                    return;
                }
            }
            if let Err(e) = File::open(&r.path) {
                r.open_error = Some(describe_open_error(&e));
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => r.exists = Some(false),
        Err(e) => {
            r.exists = Some(true);
            r.open_error = Some(describe_open_error(&e));
        }
    }
}

fn describe_open_error(e: &io::Error) -> String {
    match e.raw_os_error() {
        Some(32) => "locked by another program (sharing violation)".into(),
        Some(33) => "part of the file is locked by another program".into(),
        Some(5) => "access denied".into(),
        Some(206) | Some(3) => "path too long or invalid".into(),
        _ => e.to_string(),
    }
}

fn classify(r: &Row, is_current: bool) -> (&'static str, String) {
    if let Some(_t) = r.uploaded_at {
        return ("uploaded", "Uploaded after this to-do list was written (Backblaze hasn't re-scanned yet).".into());
    }
    if is_current {
        return ("uploading", if r.chunks_sent > 0 { format!("Uploading now: {} chunk(s) sent so far.", r.chunks_sent) } else { "Uploading now.".into() });
    }
    if r.chunks_sent > 0 {
        return ("partial", format!("Partly uploaded: {} chunk(s) of 10 MB sent; will resume.", r.chunks_sent));
    }
    if r.exists == Some(false) || r.log.as_deref().map(|l| l.contains("no longer exists")).unwrap_or(false) {
        return ("missing", "File no longer exists (deleted, renamed or moved before upload). Backblaze will drop it at the next scan.".into());
    }
    if let Some(code) = &r.skip_code {
        let msg = match code.as_str() {
            "TEMPORARY_FILE_BUSY" => "Skipped: file was in use by another program. Backblaze retries automatically.".to_string(),
            "PERMANENT_BAD_PERMISSIONS" => "Skipped: Backblaze has no permission to read this file. Not retried until permissions change.".to_string(),
            c => format!("Skipped by Backblaze: {}", c),
        };
        let st = if code.contains("PERMISSION") { "noperm" } else if code.contains("BUSY") { "busy" } else { "skipped" };
        return (st, msg);
    }
    if let Some(err) = &r.open_error {
        if err.contains("cloud-only") {
            return ("cloud", "Cloud-only placeholder (OneDrive or similar). Backblaze can't back up files that aren't downloaded.".into());
        }
        if err.contains("lock") {
            return ("locked", format!("Can't be read right now: {}.", err));
        }
        if err.contains("denied") {
            return ("noperm", "Access denied when opening the file.".into());
        }
        if err.contains("too long") {
            return ("longpath", "Path is too long or invalid.".into());
        }
        return ("error", format!("Can't open: {}", err));
    }
    if r.path.len() >= 260 && cfg!(windows) {
        return ("longpath", format!("Path is {} characters long (over the classic 260 limit).", r.path.len()));
    }
    if let (Some(cur), q) = (r.cur_mtime, r.queued_mtime) {
        if q > 0 && cur > q + 2000 {
            return ("changed", "Changed again after it was queued; Backblaze will read the newest version.".into());
        }
    }
    if let Some(log) = &r.log {
        return ("waiting", format!("Waiting. Last log message: {}", log));
    }
    if r.size > 100 * 1024 * 1024 {
        return ("waiting", "Waiting in queue (large file: uploads in 10 MB chunks).".into());
    }
    ("waiting", "Waiting in queue.".into())
}

fn update_history(dir: &Path, files: u64, bytes: u64) -> Vec<HistPoint> {
    let _ = fs::create_dir_all(dir);
    let p = dir.join("remaining_history.csv");
    let mut pts: Vec<HistPoint> = Vec::new();
    if let Some(t) = read_text(&p) {
        for l in t.lines() {
            let f: Vec<&str> = l.split(',').collect();
            if f.len() == 3 {
                if let (Ok(t), Ok(fi), Ok(b)) = (f[0].parse(), f[1].parse(), f[2].parse()) {
                    pts.push(HistPoint { t, files: fi, bytes: b });
                }
            }
        }
    }
    let now = now_ms();
    let changed = pts.last().map(|l| l.files != files || l.bytes != bytes || now - l.t > 30 * 60_000).unwrap_or(true);
    if changed {
        pts.push(HistPoint { t: now, files, bytes });
        use std::io::Write;
        if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&p) {
            let _ = writeln!(f, "{},{},{}", now, files, bytes);
        }
    }
    pts
}

/// Opens the system file manager with the file selected.
pub fn reveal(path: &str) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("explorer.exe").raw_arg(format!("/select,\"{}\"", path)).spawn()?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg("-R").arg(path).spawn()?;
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        let parent = Path::new(path).parent().map(|p| p.to_path_buf()).unwrap_or_default();
        std::process::Command::new("xdg-open").arg(parent).spawn()?;
    }
    Ok(())
}
