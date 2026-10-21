//! Platform-specific code, checked on every platform the CI builds for (Linux and Windows runners).

use crate::attachments::safe_name;

#[test]
fn free_space_is_known_for_the_temp_folder_and_for_a_folder_that_doesnt_exist_yet() {
    let tmp = std::env::temp_dir();
    let free = crate::migrate::free_space(&tmp).expect("the temp folder's volume reports its free space");
    assert!(free > 0);
    // A folder that isn't there yet is judged by its nearest existing parent.
    let planned = tmp.join(format!("gm-not-yet-{}", uuid::Uuid::new_v4())).join("vault");
    assert!(crate::migrate::free_space(&planned).is_some());
}

#[test]
fn the_default_data_folder_is_absolute_and_per_user() {
    let d = std::path::PathBuf::from(crate::storage::default_data_dir());
    assert!(d.is_absolute(), "{d:?}");
    assert_eq!(d.file_name().and_then(|f| f.to_str()), Some("graphite-mail"));
    if cfg!(windows) {
        assert!(d.to_string_lossy().to_ascii_lowercase().contains("appdata\\local"), "local, not roaming: {d:?}");
    }
}

#[test]
fn attachment_names_are_safe_on_windows_too() {
    assert_eq!(safe_name("report: Q3 <final>?.pdf"), "report_ Q3 _final__.pdf");
    assert_eq!(safe_name("a|b*c\"d.txt"), "a_b_c_d.txt");
    assert_eq!(safe_name("notes. "), "notes", "Windows drops trailing dots and spaces, so we do first");
    assert_eq!(safe_name("CON"), "_CON");
    assert_eq!(safe_name("nul.txt"), "_nul.txt");
    assert_eq!(safe_name("COM1.log"), "_COM1.log");
    assert_eq!(safe_name("lpt9"), "_lpt9");
    assert_eq!(safe_name("COM10.txt"), "COM10.txt", "only COM1-9 / LPT1-9 are devices");
    assert_eq!(safe_name("console.txt"), "console.txt");
    assert_eq!(safe_name("Résumé – 2026.docx"), "Résumé – 2026.docx", "ordinary names are left alone");
}

/// Windows paths: a drive letter is kept through normalisation and a folder inside another is recognised.
#[cfg(windows)]
#[test]
fn windows_paths_keep_their_drive_letter() {
    let tmp = std::env::temp_dir().join(format!("gm-win-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let inner = tmp.join("inner").join("deeper");
    let n = crate::migrate::normalize_for_test(&inner).unwrap();
    assert!(n.starts_with(crate::migrate::normalize_for_test(&tmp).unwrap()), "{n:?}");
    assert!(n.to_string_lossy().contains(":\\") || n.to_string_lossy().starts_with("\\\\?\\"), "{n:?}");
    std::fs::remove_dir_all(&tmp).unwrap();
}

/// The keyring backend on Windows is the Credential Manager: a secret survives a round trip there.
#[cfg(windows)]
#[test]
fn secrets_go_to_the_windows_credential_manager() {
    let key = format!("gm-test-{}", uuid::Uuid::new_v4());
    let e = keyring::Entry::new("graphite-mail-test", &key).unwrap();
    e.set_password("app password 1234").unwrap();
    assert_eq!(e.get_password().unwrap(), "app password 1234");
    e.delete_credential().unwrap();
    assert!(e.get_password().is_err(), "gone after delete");
}
