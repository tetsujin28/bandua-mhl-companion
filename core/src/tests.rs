use super::*;
use crate::hash::{c4_of_bytes, Algo, Hasher};
use std::fs;
use std::path::{Path, PathBuf};

fn tmp(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../target/test-tmp")
        .join(format!("lib-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn h(algo: Algo, data: &[u8]) -> String {
    let mut x = Hasher::new(algo);
    x.update(data);
    x.finish().canonical()
}

fn write(root: &Path, rel: &str, data: &[u8]) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, data).unwrap();
}

/// Carte fictive : un clip RED (2 fichiers), un BRAW et un sidecar.
fn card(root: &Path) -> Vec<(String, Vec<u8>)> {
    let files: Vec<(String, Vec<u8>)> = vec![
        ("A001_1106AB.RDM/A001_C001_1106XY.RDC/A001_C001_1106XY_001.R3D".into(), vec![1u8; 300_000]),
        ("A001_1106AB.RDM/A001_C001_1106XY.RDC/A001_C001_1106XY.RMD".into(), b"<rmd/>".to_vec()),
        ("B001_C002.braw".into(), vec![2u8; 120_000]),
        ("B001_C002.sidecar".into(), b"side".to_vec()),
        ("Clip & co.mov".into(), vec![3u8; 5_000]),
    ];
    for (rel, d) in &files {
        write(root, rel, d);
    }
    files
}

fn mhl1(files: &[(String, Vec<u8>)], tag: &str, f: impl Fn(&[u8]) -> String) -> String {
    let mut x = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<hashlist version=\"1.1\">\n<creatorinfo><tool>Bandua 0.2.0</tool></creatorinfo>\n");
    for (rel, d) in files {
        x += &format!(
            "<hash><file>{}</file><size>{}</size><{tag}>{}</{tag}></hash>\n",
            rel.replace('&', "&amp;"),
            d.len(),
            f(d)
        );
    }
    x + "</hashlist>\n"
}

fn asc_gen(files: &[(String, Vec<u8>)]) -> String {
    let mut x = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<hashlist version=\"2.0\" xmlns=\"urn:ASC:MHL:v2.0\">\n<creatorinfo><tool version=\"1.2\">ascmhl</tool></creatorinfo>\n<hashes>\n",
    );
    for (rel, d) in files {
        x += &format!(
            "<hash><path size=\"{}\" lastmodificationdate=\"2026-11-09T10:00:00+01:00\">{}</path><xxh64 action=\"original\" hashdate=\"2026-11-09T10:00:00+01:00\">{}</xxh64></hash>\n",
            d.len(),
            rel.replace('&', "&amp;"),
            h(Algo::Xxh64, d)
        );
    }
    x += "<directoryhash><path>A001_1106AB.RDM</path><content><xxh64>0000000000000000</xxh64></content></directoryhash>\n";
    x + "</hashes>\n</hashlist>\n"
}

fn run(src: &Path) -> Report {
    let pkgs = discover(src).unwrap();
    verify(src, &pkgs, &AtomicBool::new(false), &mut |_| {})
}

#[test]
fn intact_mhl1() {
    let d = tmp("intact");
    let files = card(&d);
    fs::write(d.join("CARD_A001.mhl"), mhl1(&files, "xxhash64be", |b| h(Algo::Xxh64, b))).unwrap();
    // rapports Bandua à côté du MHL : ignorés
    fs::write(d.join("CARD_A001_log.txt"), "log").unwrap();
    fs::write(d.join(".DS_Store"), "x").unwrap();
    let r = run(&d);
    assert!(r.ok, "{:#?}", r.packages[0].issues);
    assert_eq!(r.ok_files, 5);
    assert_eq!(r.extra_total, 0);
    let p = &r.packages[0];
    assert_eq!(p.kind, Kind::Mhl1);
    assert_eq!(p.tool.as_deref(), Some("Bandua 0.2.0"));
    assert_eq!(p.clips.len(), 3);
    let txt = report::text(&r, "fr");
    assert!(txt.contains("INTACT"));
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn detects_every_kind_of_problem() {
    let d = tmp("problems");
    let files = card(&d);
    fs::write(d.join("card.mhl"), mhl1(&files, "md5", |b| h(Algo::Md5, b))).unwrap();
    // octet modifié dans le R3D (même taille)
    let r3d = d.join(&files[0].0);
    let mut data = fs::read(&r3d).unwrap();
    data[150_000] ^= 0xFF;
    fs::write(&r3d, data).unwrap();
    // BRAW supprimé, sidecar tronqué, fichier en trop
    fs::remove_file(d.join("B001_C002.braw")).unwrap();
    fs::write(d.join("B001_C002.sidecar"), b"si").unwrap();
    fs::write(d.join("en_trop.wav"), b"x").unwrap();

    let r = run(&d);
    assert!(!r.ok);
    assert_eq!(r.problems, 3);
    assert_eq!(r.ok_files, 2);
    let p = &r.packages[0];
    let st = |rel: &str| p.issues.iter().find(|i| i.rel == rel).map(|i| i.status);
    assert_eq!(st(&files[0].0), Some(Status::Corrupt));
    assert_eq!(st("B001_C002.braw"), Some(Status::Missing));
    assert_eq!(st("B001_C002.sidecar"), Some(Status::Size));
    assert_eq!(p.extra, vec!["en_trop.wav".to_string()]);
    // le clip RED est marqué abîmé, avec son nom
    let red = p.clips.iter().find(|c| c.name == "A001_C001_1106XY").unwrap();
    assert_eq!(red.status, Status::Corrupt);
    assert_eq!(red.bad, 1);
    assert_eq!(p.issues.iter().find(|i| i.status == Status::Corrupt).unwrap().clip.as_deref(), Some("A001_C001_1106XY"));
    let txt = report::text(&r, "en");
    assert!(txt.contains("[CORRUPT]") && txt.contains("[MISSING]") && txt.contains("en_trop.wav"));
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn asc_mhl_with_chain_and_generations() {
    let d = tmp("asc");
    let files = card(&d);
    let asc = d.join("ascmhl");
    fs::create_dir_all(&asc).unwrap();
    let g1 = asc_gen(&files);
    let g1_name = "0001_card_2026-11-09_090000Z.mhl";
    fs::write(asc.join(g1_name), &g1).unwrap();
    // 2e génération : une vérification ratée ne doit pas remplacer le hash d'origine
    let g2 = format!(
        "<hashlist version=\"2.0\" xmlns=\"urn:ASC:MHL:v2.0\"><hashes><hash><path size=\"5000\">Clip &amp; co.mov</path><xxh64 action=\"failed\">1111111111111111</xxh64><md5 action=\"verified\">{}</md5></hash></hashes></hashlist>",
        h(Algo::Md5, &files[4].1)
    );
    let g2_name = "0002_card_2026-11-10_090000Z.mhl";
    fs::write(asc.join(g2_name), &g2).unwrap();
    let chain = format!(
        "<?xml version=\"1.0\"?><ascmhldirectory xmlns=\"urn:ASC:MHL:DIRECTORY:v2.0\"><hashlist sequencenr=\"1\"><path>{g1_name}</path><c4>{}</c4></hashlist><hashlist sequencenr=\"2\"><path>{g2_name}</path><c4>{}</c4></hashlist></ascmhldirectory>",
        c4_of_bytes(g1.as_bytes()),
        c4_of_bytes(g2.as_bytes())
    );
    fs::write(asc.join("ascmhl_chain.xml"), &chain).unwrap();
    // un MHL 1.1 dans le même dossier : ignoré au profit de l'ASC MHL
    fs::write(d.join("card.mhl"), mhl1(&files, "xxhash64be", |b| h(Algo::Xxh64, b))).unwrap();

    let pkgs = discover(&d).unwrap();
    assert_eq!(pkgs.len(), 1);
    assert_eq!(pkgs[0].kind, Kind::Asc);
    assert_eq!(pkgs[0].generations, 2);
    assert!(!pkgs[0].chain_broken);
    let r = verify(&d, &pkgs, &AtomicBool::new(false), &mut |_| {});
    assert!(r.ok, "{:#?}", r.packages[0].issues);
    assert_eq!(r.extra_total, 0);

    // génération modifiée après coup : chaîne rompue
    fs::write(asc.join(g2_name), g2.replace("verified", "original")).unwrap();
    assert!(discover(&d).unwrap()[0].chain_broken);
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn several_cards_and_loose_notations() {
    let d = tmp("multi");
    let a = d.join("JOUR_01/CAM_A/A001");
    let b = d.join("JOUR_01/CAM_B/B001");
    let fa = card(&a);
    let fb = card(&b);
    // xxhash64 en petit-boutiste (anciens logiciels) et xxh3 (Bandua)
    let le = |x: &[u8]| {
        let be = h(Algo::Xxh64, x);
        (0..8).rev().map(|i| &be[i * 2..i * 2 + 2]).collect::<String>()
    };
    fs::write(a.join("A001.mhl"), mhl1(&fa, "xxhash64", le)).unwrap();
    fs::write(b.join("B001.mhl"), mhl1(&fb, "xxh3", |x| h(Algo::Xxh3, x))).unwrap();
    // fichier déposé directement : seul ce MHL
    assert_eq!(discover(&a.join("A001.mhl")).unwrap().len(), 1);
    let pkgs = discover(&d).unwrap();
    assert_eq!(pkgs.len(), 2);
    assert_eq!(pkgs[0].location, "JOUR_01/CAM_A/A001");
    let r = verify(&d, &pkgs, &AtomicBool::new(false), &mut |_| {});
    assert!(r.ok);
    assert_eq!(r.ok_files, 10);
    assert!(discover(&d.join("rien")).is_err());
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn cancel_stops_early() {
    let d = tmp("cancel");
    let files = card(&d);
    fs::write(d.join("c.mhl"), mhl1(&files, "sha1", |b| h(Algo::Sha1, b))).unwrap();
    let pkgs = discover(&d).unwrap();
    let flag = AtomicBool::new(false);
    let r = verify(&d, &pkgs, &flag, &mut |e| {
        if matches!(e, Event::Progress { files_done: 1, .. }) {
            flag.store(true, Ordering::Relaxed);
        }
    });
    assert!(r.cancelled && !r.ok);
    let _ = fs::remove_dir_all(&d);
}
