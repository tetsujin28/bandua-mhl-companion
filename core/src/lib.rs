//! Bandua MHL Companion : contrôle de l'intégrité des rushes reçus à partir de
//! leurs MHL. Le moteur ne fait que lire : rien n'est jamais écrit à côté des rushes.

pub mod clips;
pub mod hash;
pub mod mhl;
pub mod rawio;
pub mod report;

use chrono::{DateTime, Local};
use clips::Brand;
use hash::Hasher;
pub use mhl::{discover, Kind, Package};
use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use walkdir::WalkDir;

pub const APP_NAME: &str = "Bandua MHL Companion";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const APP_AUTHOR: &str = "Domingo Olmo Martin";

/// Tampon de lecture : 8 Mio, multiple de l'alignement exigé par les lectures sans cache.
const BUF: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Hash identique au MHL
    Ok,
    /// Présent, bonne taille, mais le MHL ne donne aucun hash utilisable
    NoHash,
    /// Hash différent : fichier abîmé ou modifié
    Corrupt,
    /// Introuvable
    Missing,
    /// Taille différente de celle notée dans le MHL
    Size,
    /// Erreur de lecture (disque, câble, droits…)
    Unreadable,
}

impl Status {
    pub fn is_problem(self) -> bool {
        !matches!(self, Status::Ok | Status::NoHash)
    }
    fn rank(self) -> u8 {
        match self {
            Status::Ok => 0,
            Status::NoHash => 1,
            Status::Size => 2,
            Status::Unreadable => 3,
            Status::Corrupt => 4,
            Status::Missing => 5,
        }
    }
}

/// Un fichier qui pose problème (ou qui n'a pas pu être contrôlé par hash).
#[derive(Debug, Clone, Serialize)]
pub struct Issue {
    pub rel: String,
    /// Clip auquel appartient le fichier (ex. dossier .RDC), s'il est reconnu
    pub clip: Option<String>,
    pub status: Status,
    pub path: PathBuf,
    pub expected_size: Option<u64>,
    pub actual_size: Option<u64>,
    pub algo: Option<String>,
    pub expected_hash: Option<String>,
    pub actual_hash: Option<String>,
    /// Message système en cas d'erreur de lecture
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClipRow {
    pub name: String,
    pub rel_path: String,
    pub brand: Brand,
    pub brand_label: String,
    pub files: usize,
    pub bytes: u64,
    pub status: Status,
    /// Nombre de fichiers du clip en erreur
    pub bad: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackageResult {
    pub id: usize,
    pub label: String,
    pub location: String,
    pub kind: Kind,
    pub root: PathBuf,
    pub manifest: PathBuf,
    pub tool: Option<String>,
    pub algos: Vec<String>,
    pub chain_broken: bool,
    pub total_files: usize,
    pub total_bytes: u64,
    pub checked: usize,
    pub ok_files: usize,
    pub no_hash: usize,
    pub issues: Vec<Issue>,
    /// Fichiers présents dans le dossier mais absents du MHL (non contrôlés), liste tronquée
    pub extra: Vec<String>,
    pub extra_total: usize,
    pub clips: Vec<ClipRow>,
    pub cancelled: bool,
    /// Toutes les lectures ont contourné le cache du système
    pub uncached: bool,
}

impl PackageResult {
    pub fn problems(&self) -> usize {
        self.issues.iter().filter(|i| i.status.is_problem()).count()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub source: PathBuf,
    pub started: DateTime<Local>,
    pub finished: DateTime<Local>,
    pub duration_s: f64,
    pub total_files: usize,
    pub total_bytes: u64,
    pub ok_files: usize,
    pub problems: usize,
    pub no_hash: usize,
    pub extra_total: usize,
    pub chain_broken: bool,
    pub cancelled: bool,
    /// Aucun problème, tout a été contrôlé
    pub ok: bool,
    pub packages: Vec<PackageResult>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Started { total_files: usize, total_bytes: u64, packages: usize },
    PackageStarted { pkg: usize },
    Progress { done: u64, total: u64, files_done: usize, total_files: usize, current: String, pkg: usize },
    Issue { pkg: usize, issue: Issue },
    PackageDone { result: PackageResult },
    Finished { report: Report },
}

struct Throttle(Instant);
impl Throttle {
    fn ready(&mut self) -> bool {
        if self.0.elapsed() >= Duration::from_millis(80) {
            self.0 = Instant::now();
            true
        } else {
            false
        }
    }
}

fn io_msg(e: &io::Error) -> String {
    e.to_string()
}

/// Contrôle les paquets donnés (dans l'ordre) en relisant chaque fichier depuis le disque.
pub fn verify(source: &Path, packages: &[Package], cancel: &AtomicBool, emit: &mut dyn FnMut(Event)) -> Report {
    let started = Local::now();
    let t0 = Instant::now();
    let total_files: usize = packages.iter().map(|p| p.total_files).sum();
    let total_bytes: u64 = packages.iter().map(|p| p.total_bytes).sum();
    emit(Event::Started { total_files, total_bytes, packages: packages.len() });

    let roots: Vec<PathBuf> = packages.iter().map(|p| p.root.clone()).collect();
    let mut buf = rawio::AlignedBuf::new(BUF);
    let mut th = Throttle(Instant::now());
    let mut done = 0u64;
    let mut files_done = 0usize;
    let mut results = vec![];
    let mut cancelled = false;

    for pkg in packages {
        emit(Event::PackageStarted { pkg: pkg.id });
        let pairs: Vec<(String, u64)> = pkg.entries.iter().map(|e| (e.rel.clone(), e.size.unwrap_or(0))).collect();
        let clips = clips::group_clips(&pairs);
        let mut clip_of: Vec<Option<usize>> = vec![None; pkg.entries.len()];
        for (ci, c) in clips.iter().enumerate() {
            for &f in &c.files {
                clip_of[f] = Some(ci);
            }
        }
        let mut statuses = vec![Status::Ok; pkg.entries.len()];
        let mut issues = vec![];
        let (mut checked, mut ok_files, mut no_hash) = (0, 0, 0);
        let mut all_uncached = true;
        let mut any_read = false;

        for (i, e) in pkg.entries.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                cancelled = true;
                break;
            }
            let path = mhl::resolve(&pkg.root, &e.rel);
            let base = done;
            let planned = e.size.unwrap_or(0);
            emit(Event::Progress { done, total: total_bytes, files_done, total_files, current: e.rel.clone(), pkg: pkg.id });

            let mut issue = Issue {
                rel: e.rel.clone(),
                clip: clip_of[i].map(|c| clips[c].name.clone()),
                status: Status::Ok,
                path: path.clone(),
                expected_size: e.size,
                actual_size: None,
                algo: e.expected.as_ref().map(|x| x.algo.label().to_string()),
                expected_hash: e.expected.as_ref().map(|x| x.value.clone()),
                actual_hash: None,
                detail: None,
            };

            match fs::metadata(&path) {
                Err(err) if err.kind() == io::ErrorKind::NotFound => issue.status = Status::Missing,
                Err(err) => {
                    issue.status = Status::Unreadable;
                    issue.detail = Some(io_msg(&err));
                }
                Ok(md) if md.is_dir() => issue.status = Status::Missing,
                Ok(md) => {
                    let len = md.len();
                    issue.actual_size = Some(len);
                    if e.size.is_some_and(|s| s != len) {
                        issue.status = Status::Size;
                    } else if let Some(exp) = &e.expected {
                        any_read = true;
                        let mut h = Hasher::new(exp.algo);
                        let res = match rawio::open_uncached(&path) {
                            Err(err) => Err(io_msg(&err)),
                            Ok((mut f, unc)) => {
                                all_uncached &= unc;
                                rawio::read_sized(&mut f, len, &mut buf, |d| {
                                    if cancel.load(Ordering::Relaxed) {
                                        return Err("cancel".into());
                                    }
                                    h.update(d);
                                    done += d.len() as u64;
                                    if th.ready() {
                                        emit(Event::Progress {
                                            done,
                                            total: total_bytes,
                                            files_done,
                                            total_files,
                                            current: e.rel.clone(),
                                            pkg: pkg.id,
                                        });
                                    }
                                    Ok(())
                                })
                                .map_err(|err| match err {
                                    rawio::ReadError::Io(e) => io_msg(&e),
                                    rawio::ReadError::Short(n) => format!("short read ({n} / {len})"),
                                    rawio::ReadError::Long => "file grew while reading".into(),
                                    rawio::ReadError::Abort(m) => m,
                                })
                            }
                        };
                        if cancel.load(Ordering::Relaxed) {
                            cancelled = true;
                            break;
                        }
                        match res {
                            Err(m) => {
                                issue.status = Status::Unreadable;
                                issue.detail = Some(m);
                            }
                            Ok(()) => {
                                let d = h.finish();
                                if !d.matches(exp) {
                                    issue.status = Status::Corrupt;
                                }
                                issue.actual_hash = Some(d.canonical());
                            }
                        }
                    } else {
                        issue.status = Status::NoHash;
                    }
                }
            }
            done = base + planned.max(issue.actual_size.filter(|_| e.size.is_none()).unwrap_or(0));
            files_done += 1;
            checked += 1;
            statuses[i] = issue.status;
            match issue.status {
                Status::Ok => ok_files += 1,
                Status::NoHash => {
                    no_hash += 1;
                    issues.push(issue);
                }
                _ => {
                    emit(Event::Issue { pkg: pkg.id, issue: issue.clone() });
                    issues.push(issue);
                }
            }
        }
        emit(Event::Progress { done, total: total_bytes, files_done, total_files, current: String::new(), pkg: pkg.id });

        let clip_rows = clips
            .iter()
            .map(|c| {
                let worst = c.files.iter().map(|&f| statuses[f]).max_by_key(|s| s.rank()).unwrap_or(Status::Ok);
                ClipRow {
                    name: c.name.clone(),
                    rel_path: c.rel_path.clone(),
                    brand: c.brand,
                    brand_label: c.brand.label().to_string(),
                    files: c.files.len(),
                    bytes: c.bytes,
                    status: worst,
                    bad: c.files.iter().filter(|&&f| statuses[f].is_problem()).count(),
                }
            })
            .collect();

        let (extra, extra_total) = if cancelled { (vec![], 0) } else { extras(pkg, &roots) };
        let result = PackageResult {
            id: pkg.id,
            label: pkg.label.clone(),
            location: pkg.location.clone(),
            kind: pkg.kind,
            root: pkg.root.clone(),
            manifest: pkg.manifest.clone(),
            tool: pkg.tool.clone(),
            algos: pkg.algos.clone(),
            chain_broken: pkg.chain_broken,
            total_files: pkg.total_files,
            total_bytes: pkg.total_bytes,
            checked,
            ok_files,
            no_hash,
            issues,
            extra,
            extra_total,
            clips: clip_rows,
            cancelled,
            uncached: any_read && all_uncached,
        };
        emit(Event::PackageDone { result: result.clone() });
        results.push(result);
        if cancelled {
            break;
        }
    }

    let problems: usize = results.iter().map(|r| r.problems()).sum();
    let checked: usize = results.iter().map(|r| r.checked).sum();
    let chain_broken = results.iter().any(|r| r.chain_broken);
    let report = Report {
        source: source.to_path_buf(),
        started,
        finished: Local::now(),
        duration_s: t0.elapsed().as_secs_f64().max(0.001),
        total_files,
        total_bytes,
        ok_files: results.iter().map(|r| r.ok_files).sum(),
        problems,
        no_hash: results.iter().map(|r| r.no_hash).sum(),
        extra_total: results.iter().map(|r| r.extra_total).sum(),
        chain_broken,
        cancelled,
        ok: !cancelled && problems == 0 && checked == total_files,
        packages: results,
    };
    emit(Event::Finished { report: report.clone() });
    report
}

const EXTRA_LIST_MAX: usize = 500;

/// Fichiers du dossier que le MHL ne mentionne pas (donc non contrôlés).
fn extras(pkg: &Package, all_roots: &[PathBuf]) -> (Vec<String>, usize) {
    let listed: HashSet<&str> = pkg.entries.iter().map(|e| e.rel.as_str()).collect();
    let stem = match pkg.kind {
        Kind::Mhl1 => pkg.manifest.file_stem().map(|s| s.to_string_lossy().into_owned()),
        Kind::Asc => None,
    };
    // dossiers d'autres paquets imbriqués sous celui-ci : contrôlés à part
    let nested: Vec<&PathBuf> = all_roots.iter().filter(|r| *r != &pkg.root && r.starts_with(&pkg.root)).collect();
    let mut list = vec![];
    let mut total = 0;
    let walker = WalkDir::new(&pkg.root).follow_links(false).sort_by_file_name().into_iter().filter_entry(|e| {
        if e.depth() == 0 {
            return true;
        }
        let n = e.file_name().to_string_lossy();
        if mhl::is_junk_name(&n) || (e.file_type().is_dir() && n.eq_ignore_ascii_case("ascmhl")) {
            return false;
        }
        !(e.file_type().is_dir() && nested.iter().any(|r| r.as_path() == e.path()))
    });
    for entry in walker.flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(&pkg.root)
            .unwrap()
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        if listed.contains(rel.as_str()) || rel.to_ascii_lowercase().ends_with(".mhl") {
            continue;
        }
        // rapports écrits par Bandua à côté de son MHL (<nom>_log.txt, <nom>_rapport.pdf…)
        if let Some(s) = &stem {
            if !rel.contains('/') && rel.starts_with(s.as_str()) {
                continue;
            }
        }
        total += 1;
        if list.len() < EXTRA_LIST_MAX {
            list.push(rel);
        }
    }
    (list, total)
}

#[cfg(test)]
mod tests;
