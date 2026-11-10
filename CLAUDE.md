# Bandua MHL Companion

Application gratuite de Domingo Olmo Martin (Dom) pour ceux qui **reçoivent** des rushes : elle relit chaque fichier, le compare au MHL qui l'accompagne et dit, clip par clip, ce qui est intact, abîmé, manquant ou incomplet. Compagnon de [Bandua](https://github.com/tetsujin28/bandua) (l'app de déchargement), mais elle doit fonctionner avec les MHL de n'importe quel logiciel (Silverstack, ShotPut Pro, Hedge, YoYotta, mhl-tool…).

Windows et macOS (binaire universel). Pas de Linux pour l'instant. Licence MIT.

## Règles de travail

- **Ne jamais lancer de fabrication GitHub Actions sans que Dom le demande explicitement.** Cela inclut « Run workflow » (`workflow_dispatch`) et la publication d'une Release, qui déclenche aussi `build.yml`.
- **Toujours passer par une branche et une pull request**, fusionnée sur GitHub (commits « Verified »). Ne jamais pousser directement sur `main`.
- **Aucun tiret cadratin (caractère U+2014)** dans les textes écrits pour Dom : interface, README, notes de version, messages de commit. Utiliser virgules, points ou parenthèses.
- Échanges avec Dom en français. Commentaires et messages de commit en français, comme le code existant.
- Interface et rapports **toujours en trois langues** (français, anglais, espagnol) : toute nouvelle chaîne s'ajoute dans les trois.
- README : `README.md` (anglais, langue principale), `README.fr.md`, `README.es.md`, à tenir synchronisés. Le README français tutoie l'utilisateur.
- Interface sombre, simple et soignée. Le vert et le rouge sont réservés aux statuts.

## Principe non négociable

**Lecture seule.** L'app ne modifie, ne renomme et n'écrit jamais rien à côté des rushes. Le seul fichier écrit est le rapport exporté, à l'endroit choisi par l'utilisateur.

## Organisation

```
core/                 companion-core (moteur, sans interface)
  src/lib.rs          verify(), Status, Report, événements de progression
  src/mhl.rs          discover(), parse_mhl1() (MHL 1.1), parse_asc() (ASC MHL + ascmhl_chain.xml)
  src/hash.rs         xxHash64, XXH3, XXH128, xxHash32, MD5, SHA-1, SHA-256, C4 (le plus rapide disponible est utilisé)
  src/rawio.rs        lectures sans cache système, tampon aligné de 8 Mio, lectures bornées à la taille attendue
  src/clips.rs        regroupement des fichiers en clips par marque (RED .RDC, ARRI, BRAW, Sony, Canon, GoPro, DJI)
  src/report.rs       rapport texte et libellés FR / EN / ES
  src/tests.rs        tests unitaires (module de lib.rs)
  src/bin/cli.rs      bandua-mhl-check <dossier|fichier.mhl> [--lang fr|en|es] [--report f.txt]
src-tauri/            application Tauri 2 (src/main.rs : commandes scan, start_check, cancel_check, export_report…)
ui/                   index.html, app.js, i18n.js : HTML/CSS/JS sans npm ni framework
.github/workflows/build.yml   fabrication Windows + macOS universel
```

L'interface parle au moteur par `invoke` et reçoit la progression par l'événement `check`. Sans `window.__TAURI__`, `ui/` tourne en **mode démo** dans un navigateur avec des données simulées, pratique pour tester l'interface.

Statuts : `Ok` (INTACT), `Corrupt` (ABÎMÉ), `Missing` (MANQUANT), `Size` (INCOMPLET, taille différente), `Unreadable` (ILLISIBLE), `NoHash` (SANS HASH, taille seule).
CLI : code de sortie 0 tout est intact, 1 problèmes trouvés, 2 aucun MHL.

## Commandes

```
cargo test -p companion-core          # à lancer avant chaque PR
cargo run -p companion-core --bin bandua-mhl-check -- <dossier> --lang fr
cargo install tauri-cli --version "^2" --locked
cd src-tauri && cargo tauri dev       # lancer l'app
```

## Publier une version

Changer le numéro dans `core/Cargo.toml`, `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json` et la constante `VERSION` de `ui/app.js`. Dom crée ensuite la Release `vX.Y.Z` lui-même. Les fichiers produits : `Bandua_MHL_Companion_x.y.z_portable.exe`, `Bandua.MHL.Companion_x.y.z_x64-setup.exe` / `.msi`, `Bandua.MHL.Companion_x.y.z_universal.dmg`.

## Code partagé avec Bandua

`hash.rs`, `rawio.rs` et `clips.rs` sont dérivés de ceux de Bandua. Une correction de fiabilité faite ici (cache, lectures bornées, reconnaissance des clips) est probablement à reporter dans Bandua, et inversement : le signaler à Dom.
