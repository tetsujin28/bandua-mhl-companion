//! Lectures qui contournent le cache du système.
//!
//! Sans cela, la « vérification » d'une copie fraîchement écrite relit souvent
//! la mémoire vive et non le disque : un disque ou un câble défaillant passerait
//! inaperçu. Selon le système :
//! - Windows : ouverture avec FILE_FLAG_NO_BUFFERING (tampon et tailles alignés) ;
//! - macOS : F_NOCACHE sur les descripteurs de lecture et d'écriture ;
//! - Linux : après synchronisation, posix_fadvise(DONTNEED) vide les pages du
//!   fichier du cache avant la relecture.
//!
//! Les lectures sont toujours bornées à la taille attendue du fichier : on ne lit
//! jamais au-delà, ce qui garde les accès alignés en mode sans cache et permet
//! de repérer un fichier plus court ou plus long que prévu.

use std::alloc::{alloc_zeroed, dealloc, handle_alloc_error, Layout};
use std::fs::File;
use std::io::{self, Read};
use std::ops::{Deref, DerefMut};
use std::path::Path;

/// Alignement du tampon : couvre les secteurs de 512 o et 4 Ko et les pages de 16 Ko.
pub const ALIGN: usize = 64 * 1024;

/// Tampon aligné, exigé par les lectures sans cache (Windows, macOS).
pub struct AlignedBuf {
    ptr: *mut u8,
    len: usize,
}

// Le tampon possède sa mémoire, comme un Vec<u8>.
unsafe impl Send for AlignedBuf {}

impl AlignedBuf {
    pub fn new(len: usize) -> Self {
        assert!(len > 0 && len.is_multiple_of(ALIGN), "taille de tampon non alignée");
        let layout = Layout::from_size_align(len, ALIGN).expect("layout");
        let ptr = unsafe { alloc_zeroed(layout) };
        if ptr.is_null() {
            handle_alloc_error(layout);
        }
        AlignedBuf { ptr, len }
    }
}

impl Deref for AlignedBuf {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

impl DerefMut for AlignedBuf {
    fn deref_mut(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
    }
}

impl Drop for AlignedBuf {
    fn drop(&mut self) {
        unsafe { dealloc(self.ptr, Layout::from_size_align_unchecked(self.len, ALIGN)) }
    }
}

/// Ouvre un fichier en lecture en contournant le cache du système.
/// Le booléen vaut `true` si le contournement est effectif sur ce volume.
pub fn open_uncached(path: &Path) -> io::Result<(File, bool)> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_NO_BUFFERING: u32 = 0x2000_0000;
        match std::fs::OpenOptions::new().read(true).custom_flags(FILE_FLAG_NO_BUFFERING).open(path) {
            Ok(f) => Ok((f, true)),
            // Certains volumes (réseau, pilotes tiers) refusent le mode sans cache
            Err(_) => File::open(path).map(|f| (f, false)),
        }
    }
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        let f = File::open(path)?;
        let ok = set_nocache(&f);
        Ok((f, ok))
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        let f = File::open(path)?;
        let ok = drop_cache(&f);
        Ok((f, ok))
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "ios", target_os = "linux", target_os = "android")))]
    {
        File::open(path).map(|f| (f, false))
    }
}

/// À appeler sur un fichier ouvert en écriture, avant d'écrire.
/// macOS : les pages écrites ne restent pas dans le cache. Ailleurs : rien.
pub fn prepare_write(_f: &File) {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        let _ = set_nocache(_f);
    }
}

/// Vide du cache les pages (déjà synchronisées) d'un fichier.
/// Linux uniquement ; renvoie `false` ailleurs ou en cas d'échec.
pub fn drop_cache(_f: &File) -> bool {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        use std::os::unix::io::AsRawFd;
        unsafe { libc::posix_fadvise(_f.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED) == 0 }
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        false
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn set_nocache(f: &File) -> bool {
    use std::os::unix::io::AsRawFd;
    unsafe { libc::fcntl(f.as_raw_fd(), libc::F_NOCACHE, 1) != -1 }
}

/// Problème rencontré pendant une lecture bornée.
#[derive(Debug)]
pub enum ReadError {
    Io(io::Error),
    /// Fichier plus court que prévu : octets réellement lus
    Short(u64),
    /// Fichier plus long que prévu
    Long,
    /// Interruption demandée par le traitement (annulation, erreur d'écriture…)
    Abort(String),
}

/// Lit exactement `size` octets depuis `f`, sans jamais demander de données
/// au-delà, et passe chaque bloc à `chunk`. Vérifie ensuite que la taille du
/// fichier n'a pas changé.
pub fn read_sized(
    f: &mut File,
    size: u64,
    buf: &mut [u8],
    mut chunk: impl FnMut(&[u8]) -> Result<(), String>,
) -> Result<(), ReadError> {
    let mut total = 0u64;
    while total < size {
        // On demande toujours le tampon entier (taille alignée) ; le système
        // renvoie moins à la fin du fichier.
        let n = match f.read(buf) {
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(ReadError::Io(e)),
        };
        if n == 0 {
            return Err(ReadError::Short(total));
        }
        if total + n as u64 > size {
            return Err(ReadError::Long);
        }
        chunk(&buf[..n]).map_err(ReadError::Abort)?;
        total += n as u64;
    }
    match f.metadata() {
        Ok(md) if md.len() > size => Err(ReadError::Long),
        Ok(md) if md.len() < size => Err(ReadError::Short(md.len())),
        Ok(_) => Ok(()),
        Err(e) => Err(ReadError::Io(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Dossier de test sur le disque du projet (pas /tmp, souvent en mémoire)
    fn tmp(name: &str) -> std::path::PathBuf {
        let d = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../target/test-tmp")
            .join(format!("rawio-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn aligned_buffer() {
        let b = AlignedBuf::new(ALIGN * 4);
        assert_eq!(b.as_ptr() as usize % ALIGN, 0);
        assert_eq!(b.len(), ALIGN * 4);
    }

    #[test]
    fn sized_read_detects_short_and_long() {
        let d = tmp("sized");
        let p = d.join("f.bin");
        std::fs::write(&p, vec![7u8; 100_000]).unwrap();
        let mut buf = AlignedBuf::new(ALIGN);

        let (mut f, _) = open_uncached(&p).unwrap();
        let mut got = 0usize;
        read_sized(&mut f, 100_000, &mut buf, |c| {
            got += c.len();
            Ok(())
        })
        .unwrap();
        assert_eq!(got, 100_000);

        let (mut f, _) = open_uncached(&p).unwrap();
        assert!(matches!(read_sized(&mut f, 150_000, &mut buf, |_| Ok(())), Err(ReadError::Short(100_000))));

        let (mut f, _) = open_uncached(&p).unwrap();
        assert!(matches!(read_sized(&mut f, 99_999, &mut buf, |_| Ok(())), Err(ReadError::Long)));

        // fichier vide
        let e = d.join("vide");
        std::fs::File::create(&e).unwrap();
        let (mut f, _) = open_uncached(&e).unwrap();
        read_sized(&mut f, 0, &mut buf, |_| Ok(())).unwrap();
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Linux : après synchronisation et ouverture sans cache, aucune page
    /// du fichier ne doit rester en mémoire (vérifié avec mincore).
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_pages_evicted() {
        use std::os::unix::io::AsRawFd;
        let d = tmp("evict");
        let p = d.join("f.bin");
        let size = 4 * 1024 * 1024;
        {
            let mut f = File::create(&p).unwrap();
            f.write_all(&vec![1u8; size]).unwrap();
            f.sync_all().unwrap();
        }
        let (f, ok) = open_uncached(&p).unwrap();
        assert!(ok);
        let resident = unsafe {
            let m = libc::mmap(std::ptr::null_mut(), size, libc::PROT_READ, libc::MAP_SHARED, f.as_raw_fd(), 0);
            assert_ne!(m, libc::MAP_FAILED);
            let page = libc::sysconf(libc::_SC_PAGESIZE) as usize;
            let mut vec = vec![0u8; size.div_ceil(page)];
            assert_eq!(libc::mincore(m, size, vec.as_mut_ptr()), 0);
            libc::munmap(m, size);
            vec.iter().filter(|&&b| b & 1 == 1).count()
        };
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!(resident, 0, "{resident} pages encore dans le cache après ouverture sans cache");
    }
}
