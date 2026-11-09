//! Contrôle en ligne de commande : `bandua-mhl-check <dossier|fichier.mhl> [--lang fr|en|es] [--report fichier.txt]`
//! Code de sortie : 0 si tout est intact, 1 en cas de problème, 2 si aucun MHL n'est trouvé.

use companion_core::{discover, report, verify, Event};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

fn main() {
    let mut args = std::env::args().skip(1);
    let mut path = None;
    let mut lang = "fr".to_string();
    let mut out = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--lang" => lang = args.next().unwrap_or(lang),
            "--report" => out = args.next().map(PathBuf::from),
            _ => path = Some(PathBuf::from(a)),
        }
    }
    let Some(path) = path else {
        eprintln!("usage: bandua-mhl-check <folder|file.mhl> [--lang fr|en|es] [--report out.txt]");
        std::process::exit(2);
    };
    let pkgs = match discover(&path) {
        Ok(p) if !p.is_empty() => p,
        Ok(_) => {
            eprintln!("No MHL found in {}", path.display());
            std::process::exit(2);
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    for p in &pkgs {
        eprintln!("· {} ({} files, {})", p.label, p.total_files, report::human_bytes(&lang, p.total_bytes));
    }
    let r = verify(&path, &pkgs, &AtomicBool::new(false), &mut |e| {
        if let Event::Issue { issue, .. } = e {
            eprintln!("  ✗ [{}] {}", report::status_label(&lang, issue.status), issue.rel);
        }
    });
    let txt = report::text(&r, &lang);
    match out {
        Some(o) => std::fs::write(&o, txt).expect("report"),
        None => print!("{txt}"),
    }
    std::process::exit(if r.ok { 0 } else { 1 });
}
