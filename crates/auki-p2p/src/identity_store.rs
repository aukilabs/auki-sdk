use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use tempfile::{Builder, NamedTempFile};
use zeroize::Zeroizing;

use crate::{Error, Identity, Result};

// Canonical Ed25519 private-key protobufs are currently 68 bytes. Keep a
// generous fixed ceiling so corrupt or hostile local files cannot trigger an
// unbounded allocation while leaving room for compatible encoding changes.
const MAX_IDENTITY_FILE_BYTES: u64 = 1_024;

impl Identity {
    /// Load a stable Ed25519 libp2p identity, creating it only if `path` is absent.
    ///
    /// Existing material must be a canonical libp2p private-key protobuf and,
    /// on Unix, must have exactly `0o600` permissions. Invalid or unsafe
    /// existing material is returned as an error and is never replaced.
    /// Concurrent creators converge on the identity that wins the atomic
    /// no-clobber publish race.
    pub fn load_or_create(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        match open_existing(path)? {
            Some(identity) => Ok(identity),
            None => create_identity(path),
        }
    }
}

fn open_existing(path: &Path) -> Result<Option<Identity>> {
    let file = match open_identity_file(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) if is_symlink_error(&error) => return Err(Error::IdentityFileSymlink),
        Err(error) => return Err(error.into()),
    };
    load_open_file(file).map(Some)
}

fn load_existing(path: &Path) -> Result<Identity> {
    let file = open_identity_file(path).map_err(|error| {
        if is_symlink_error(&error) {
            Error::IdentityFileSymlink
        } else {
            error.into()
        }
    })?;
    load_open_file(file)
}

#[cfg(unix)]
fn open_identity_file(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        // `O_NOFOLLOW` rejects a final symlink. `O_NONBLOCK` prevents a
        // configured FIFO or device from hanging startup before the metadata
        // check rejects it as non-regular; it has no effect on regular files.
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
}

#[cfg(not(unix))]
fn open_identity_file(path: &Path) -> io::Result<File> {
    OpenOptions::new().read(true).open(path)
}

#[cfg(unix)]
fn is_symlink_error(error: &io::Error) -> bool {
    error.raw_os_error() == Some(libc::ELOOP)
}

#[cfg(not(unix))]
fn is_symlink_error(_error: &io::Error) -> bool {
    false
}

fn load_open_file(mut file: File) -> Result<Identity> {
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(Error::IdentityFileNotRegular);
    }
    validate_permissions(&metadata)?;

    if metadata.len() > MAX_IDENTITY_FILE_BYTES {
        return Err(Error::IdentityFileTooLarge {
            actual: metadata.len(),
            maximum: MAX_IDENTITY_FILE_BYTES,
        });
    }

    let mut encoded = Zeroizing::new(Vec::with_capacity(metadata.len() as usize));
    Read::by_ref(&mut file)
        .take(MAX_IDENTITY_FILE_BYTES + 1)
        .read_to_end(&mut encoded)?;
    if encoded.len() as u64 > MAX_IDENTITY_FILE_BYTES {
        return Err(Error::IdentityFileTooLarge {
            actual: encoded.len() as u64,
            maximum: MAX_IDENTITY_FILE_BYTES,
        });
    }

    Identity::from_protobuf_encoding(&encoded)
}

fn create_identity(path: &Path) -> Result<Identity> {
    let parent = parent_directory(path);
    fs::create_dir_all(&parent)?;

    let identity = Identity::generate();
    publish_identity(path, &parent, identity)
}

fn publish_identity(path: &Path, parent: &Path, identity: Identity) -> Result<Identity> {
    let encoded = Zeroizing::new(identity.to_protobuf_encoding()?);
    let mut temporary = new_secret_tempfile(parent)?;
    temporary.write_all(&encoded)?;
    temporary.as_file().sync_all()?;

    match temporary.persist_noclobber(path) {
        Ok(_persisted) => {
            sync_directory(parent)?;
            Ok(identity)
        }
        Err(error) => {
            let tempfile::PersistError { error, file } = error;
            // Ensure the losing candidate is removed before loading the winner.
            drop(file);
            if error.kind() == io::ErrorKind::AlreadyExists {
                let winner = load_existing(path)?;
                sync_directory(parent)?;
                Ok(winner)
            } else {
                Err(error.into())
            }
        }
    }
}

fn new_secret_tempfile(parent: &Path) -> io::Result<NamedTempFile> {
    let file = Builder::new()
        .prefix(".auki-p2p-identity-")
        .tempfile_in(parent)?;
    set_owner_only_permissions(file.as_file())?;
    Ok(file)
}

fn parent_directory(path: &Path) -> PathBuf {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

#[cfg(unix)]
fn validate_permissions(metadata: &fs::Metadata) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mode = metadata.permissions().mode() & 0o7777;
    if mode != 0o600 {
        return Err(Error::InsecureIdentityFilePermissions { mode });
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_permissions(_metadata: &fs::Metadata) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_owner_only_permissions(file: &File) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    file.set_permissions(fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_owner_only_permissions(_file: &File) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};
    use std::thread;

    use super::*;

    fn write_existing(path: &Path, bytes: &[u8]) {
        fs::write(path, bytes).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        }
    }

    #[test]
    fn creates_canonical_identity_and_restores_the_same_peer() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nested").join("peer.identity");

        let created = Identity::load_or_create(&path).unwrap();
        let persisted = fs::read(&path).unwrap();
        let decoded = Identity::from_protobuf_encoding(&persisted).unwrap();
        let restored = Identity::load_or_create(&path).unwrap();

        assert_eq!(decoded.peer_id(), created.peer_id());
        assert_eq!(restored.peer_id(), created.peer_id());
        assert_eq!(persisted, created.to_protobuf_encoding().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn creates_identity_with_exact_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("peer.identity");
        Identity::load_or_create(&path).unwrap();

        let mode = fs::metadata(path).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn corrupt_existing_identity_is_rejected_without_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("peer.identity");
        let corrupt = b"not-a-libp2p-private-key";
        write_existing(&path, corrupt);

        assert!(matches!(
            Identity::load_or_create(&path),
            Err(Error::InvalidIdentityPrivateKey)
        ));
        assert_eq!(fs::read(path).unwrap(), corrupt);
    }

    #[test]
    fn noncanonical_existing_identity_is_rejected_without_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("peer.identity");
        let mut noncanonical = Identity::from_ed25519_seed(&[0x31; 32])
            .to_protobuf_encoding()
            .unwrap();
        noncanonical.extend_from_slice(&[0x18, 0x00]);
        write_existing(&path, &noncanonical);

        assert!(matches!(
            Identity::load_or_create(&path),
            Err(Error::InvalidIdentityPrivateKey)
        ));
        assert_eq!(fs::read(path).unwrap(), noncanonical);
    }

    #[test]
    fn wrong_algorithm_existing_identity_is_rejected_without_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("peer.identity");
        let mut secp256k1 = vec![0x08, 0x02, 0x12, 0x20];
        secp256k1.extend_from_slice(&[1; 32]);
        write_existing(&path, &secp256k1);

        assert!(Identity::load_or_create(&path).is_err());
        assert_eq!(fs::read(path).unwrap(), secp256k1);
    }

    #[test]
    fn oversized_existing_identity_is_rejected_without_unbounded_read() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("peer.identity");
        let oversized = vec![0u8; MAX_IDENTITY_FILE_BYTES as usize + 1];
        write_existing(&path, &oversized);

        assert!(matches!(
            Identity::load_or_create(&path),
            Err(Error::IdentityFileTooLarge { .. })
        ));
        assert_eq!(fs::read(path).unwrap(), oversized);
    }

    #[cfg(unix)]
    #[test]
    fn insecure_existing_permissions_are_rejected_without_replacement() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("peer.identity");
        let encoded = Identity::from_ed25519_seed(&[0x42; 32])
            .to_protobuf_encoding()
            .unwrap();
        fs::write(&path, &encoded).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        assert!(matches!(
            Identity::load_or_create(&path),
            Err(Error::InsecureIdentityFilePermissions { mode: 0o644 })
        ));
        assert_eq!(fs::read(path).unwrap(), encoded);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_identity_path_is_rejected_without_following_or_replacing_it() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target.identity");
        let path = directory.path().join("peer.identity");
        let encoded = Identity::from_ed25519_seed(&[0x43; 32])
            .to_protobuf_encoding()
            .unwrap();
        write_existing(&target, &encoded);
        symlink(&target, &path).unwrap();

        assert!(matches!(
            Identity::load_or_create(&path),
            Err(Error::IdentityFileSymlink)
        ));
        assert!(fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(target).unwrap(), encoded);
    }

    #[test]
    fn publish_conflict_loads_the_existing_winner_without_overwrite() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("peer.identity");
        let winner = Identity::from_ed25519_seed(&[0x51; 32]);
        let loser = Identity::from_ed25519_seed(&[0x52; 32]);
        let winner_bytes = winner.to_protobuf_encoding().unwrap();
        write_existing(&path, &winner_bytes);

        let selected = publish_identity(&path, directory.path(), loser).unwrap();

        assert_eq!(selected.peer_id(), winner.peer_id());
        assert_eq!(fs::read(path).unwrap(), winner_bytes);
    }

    #[test]
    fn publish_conflict_rejects_a_corrupt_winner_without_overwrite() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("peer.identity");
        let corrupt = b"winner-is-corrupt";
        write_existing(&path, corrupt);

        assert!(matches!(
            publish_identity(&path, directory.path(), Identity::generate()),
            Err(Error::InvalidIdentityPrivateKey)
        ));
        assert_eq!(fs::read(&path).unwrap(), corrupt);

        let entries: Vec<_> = fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(entries, ["peer.identity"]);
    }

    #[test]
    fn concurrent_creators_converge_on_one_persisted_identity() {
        const CREATOR_COUNT: usize = 16;

        let directory = tempfile::tempdir().unwrap();
        let path = Arc::new(directory.path().join("peer.identity"));
        let barrier = Arc::new(Barrier::new(CREATOR_COUNT));
        let handles: Vec<_> = (0..CREATOR_COUNT)
            .map(|_| {
                let path = Arc::clone(&path);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    Identity::load_or_create(path.as_ref()).map(|identity| identity.peer_id())
                })
            })
            .collect();
        let peer_ids: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap().unwrap())
            .collect();

        assert!(peer_ids.iter().all(|peer_id| peer_id == &peer_ids[0]));
        let persisted =
            Identity::from_protobuf_encoding(&fs::read(path.as_ref()).unwrap()).unwrap();
        assert_eq!(persisted.peer_id(), peer_ids[0]);

        let entries: Vec<_> = fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(entries, ["peer.identity"]);
    }

    #[test]
    fn non_regular_existing_path_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("peer.identity");
        fs::create_dir(&path).unwrap();

        assert!(matches!(
            Identity::load_or_create(path),
            Err(Error::IdentityFileNotRegular)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn fifo_identity_path_is_rejected_without_blocking() {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("peer.identity");
        let encoded_path = CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: `encoded_path` is a live, NUL-terminated filesystem path and
        // the mode argument has the platform `mode_t` representation.
        let result = unsafe { libc::mkfifo(encoded_path.as_ptr(), 0o600) };
        assert_eq!(result, 0, "mkfifo failed: {}", io::Error::last_os_error());

        assert!(matches!(
            Identity::load_or_create(path),
            Err(Error::IdentityFileNotRegular)
        ));
    }
}
