use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use stratamon_core::config::*;

static NEXT: AtomicU32 = AtomicU32::new(0);

struct Folder(PathBuf);

impl Folder {
    fn new() -> Self {
        let name = format!("stratamon-config-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
        let path = std::env::temp_dir().join(name);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn config(&self) -> PathBuf {
        self.0.join("config.json")
    }

    fn load_text(&self, json: &str) -> AppConfig {
        std::fs::write(self.config(), json).unwrap();
        load(&self.config()).0
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_saved_config_loads_back_unchanged() {
    let folder = Folder::new();
    let saved = AppConfig {
        servers: vec![ServerConfig {
            id: "a".into(),
            name: "NEDM".into(),
            addresses: vec!["192.168.1.5".into(), "100.1.2.3".into()],
            key: "secret".into(),
        }],
        alerts: AlertThresholds { gpu_temperature_c: 70.0, ram_percent: 80.0, queue_wait_seconds: 20, sustain_seconds: 30 },
        poll_seconds: 7,
        desktop_notifications: false,
    };

    save(&folder.config(), &saved).unwrap();
    let (loaded, moved_to) = load(&folder.config());

    assert_eq!(moved_to, None);
    assert_eq!(loaded, saved);
}

#[test]
fn no_file_means_defaults() {
    let folder = Folder::new();
    let (config, moved_to) = load(&folder.config());

    assert_eq!(moved_to, None);
    assert!(config.servers.is_empty());
    assert_eq!(config.alerts, AlertThresholds::default());
    assert_eq!(config.poll_seconds, 3);
    assert!(config.desktop_notifications);
}

// Exactly what an earlier build saved: old limits that no longer exist, and none of the new ones.
#[test]
fn settings_a_file_does_not_mention_keep_their_defaults_rather_than_becoming_zero() {
    let config = Folder::new().load_text(
        r#"{
          "servers": [
            { "id": "0123", "name": "NEDM", "addresses": ["192.168.1.5"], "key": "secret" }
          ],
          "alerts": { "cpuPercent": 90, "memoryPercent": 90, "diskPercent": 90, "cpuTemperatureC": 85, "gpuTemperatureC": 85, "sustainSeconds": 45 },
          "pollSeconds": 3,
          "desktopNotifications": false
        }"#,
    );

    assert_eq!(config.alerts.ram_percent, 95.0);
    assert_eq!(config.alerts.queue_wait_seconds, 60);
    assert_eq!(config.alerts.gpu_temperature_c, 85.0);
    assert_eq!(config.alerts.sustain_seconds, 45);
    assert!(!config.desktop_notifications);
    assert_eq!(config.servers[0].key, "secret");
}

// The exact text the C# build writes, so a key saved by it keeps working.
#[test]
fn reads_a_file_written_by_the_csharp_build() {
    let config = Folder::new().load_text(
        r#"{
  "servers": [
    {
      "id": "6d2f0c1f9a5b4e7e8a2b1c3d4e5f6071",
      "name": "NEDM2",
      "addresses": [
        "192.168.1.231"
      ],
      "key": "sk-strata-example"
    }
  ],
  "alerts": {
    "gpuTemperatureC": 85,
    "ramPercent": 95,
    "queueWaitSeconds": 60,
    "sustainSeconds": 45
  },
  "pollSeconds": 3,
  "desktopNotifications": false
}"#,
    );

    assert_eq!(config.servers[0].name, "NEDM2");
    assert_eq!(config.servers[0].addresses, ["192.168.1.231"]);
    assert_eq!(config.servers[0].key, "sk-strata-example");
    assert_eq!(config.alerts.sustain_seconds, 45);
}

#[test]
fn an_empty_object_is_all_defaults() {
    assert_eq!(Folder::new().load_text("{}").alerts, AlertThresholds::default());
    assert_eq!(Folder::new().load_text("{}").poll_seconds, 3);
    assert!(Folder::new().load_text("{}").desktop_notifications);
}

#[test]
fn a_server_entry_with_only_a_name_is_still_usable() {
    let config = Folder::new().load_text(r#"{ "servers": [ { "name": "NEDM" } ] }"#);
    let server = &config.servers[0];

    assert_eq!(server.name, "NEDM");
    assert!(server.addresses.is_empty());
    assert_eq!(server.key, "");
    assert!(!server.id.is_empty());
}

#[test]
fn new_servers_get_different_ids() {
    assert_ne!(ServerConfig::default().id, ServerConfig::default().id);
}

// The file itself is sealed on Windows, so the names are checked on the text the seal keeps:
// the same text older builds wrote and any build reads, and the one property names live in.
#[test]
fn files_keep_the_same_property_names_so_settings_survive_an_update() {
    let config = AppConfig { servers: vec![ServerConfig { name: "NEDM".into(), ..Default::default() }], ..Default::default() };
    let text = serde_json::to_string_pretty(&config).unwrap();

    for name in ["servers", "name", "addresses", "key", "alerts", "gpuTemperatureC", "ramPercent", "queueWaitSeconds", "sustainSeconds", "pollSeconds", "desktopNotifications"] {
        assert!(text.contains(&format!("\"{name}\"")), "{name}");
    }
}

#[test]
fn a_corrupt_file_is_kept_rather_than_overwritten() {
    let folder = Folder::new();
    std::fs::write(folder.config(), "{ this is not json").unwrap();

    let (config, moved_to) = load(&folder.config());

    assert!(config.servers.is_empty());
    assert!(!folder.config().exists());
    let moved_to = moved_to.unwrap();
    assert!(moved_to.to_string_lossy().contains("config.json.corrupt-"));
    assert_eq!(std::fs::read_to_string(moved_to).unwrap(), "{ this is not json");
}

#[test]
fn a_file_with_a_wrong_type_is_treated_as_corrupt_rather_than_crashing() {
    let folder = Folder::new();
    std::fs::write(folder.config(), r#"{ "pollSeconds": "fast" }"#).unwrap();
    assert!(load(&folder.config()).1.is_some());
}

#[test]
fn a_file_that_isnt_text_at_all_is_read_as_nothing_but_kept_aside() {
    let folder = Folder::new();
    std::fs::write(folder.config(), [0x00, 0x01, 0x02, 0xfe, 0xff]).unwrap();

    let (config, moved_to) = load(&folder.config());

    assert!(config.servers.is_empty());
    // Renamed like any other unreadable file, so the next save can't overwrite the bytes
    // without leaving a copy of them somewhere.
    assert!(moved_to.is_some());
    assert!(!folder.config().exists());
}

// A seal this build can't open: the bytes are the user's keys, kept aside like a corrupt
// file rather than read or replaced. The same test runs everywhere: it's a seal nowhere.
#[test]
fn a_seal_this_build_cant_open_is_kept_rather_than_overwritten() {
    let folder = Folder::new();
    let bytes = b"SMONCFIG\x01garbage-not-a-real-seal";
    std::fs::write(folder.config(), bytes).unwrap();

    let (config, moved_to) = load(&folder.config());

    assert!(config.servers.is_empty());
    assert!(!folder.config().exists());
    let moved_to = moved_to.unwrap();
    assert!(moved_to.to_string_lossy().contains("config.json.corrupt-"));
    assert_eq!(std::fs::read(moved_to).unwrap(), bytes);
}

#[test]
fn a_seal_from_a_later_version_is_kept_rather_than_overwritten() {
    let folder = Folder::new();
    std::fs::write(folder.config(), b"SMONCFIG\x02whatever-a-later-build-wrote").unwrap();
    assert!(load(&folder.config()).1.is_some());
}

#[cfg(windows)]
#[test]
fn a_saved_file_holds_its_names_and_keys_as_sealed_bytes() {
    let folder = Folder::new();
    let config = AppConfig {
        servers: vec![ServerConfig { id: "a".into(), name: "NEDM".into(), addresses: vec!["192.168.1.5".into()], key: "sk-strata-secret".into() }],
        ..Default::default()
    };

    save(&folder.config(), &config).unwrap();
    let bytes = std::fs::read(folder.config()).unwrap();

    assert!(bytes.starts_with(br"SMONCFIG"), "the file is a seal");
    let text = String::from_utf8_lossy(&bytes);
    assert!(!text.contains("sk-strata-secret"), "the key isn't plain text");
    assert!(!text.contains("NEDM"), "the names aren't plain text either");
    assert_eq!(load(&folder.config()).0.servers[0].key, "sk-strata-secret", "and it still reads back");
}

#[cfg(windows)]
#[test]
fn a_plain_file_gets_sealed_the_first_time_its_saved_through() {
    let folder = Folder::new();
    let json = r#"{ "servers": [{ "name": "NEDM", "addresses": ["192.168.1.5"], "key": "sk-strata-secret" }] }"#;
    std::fs::write(folder.config(), json).unwrap();

    assert!(needs_sealing(&folder.config()), "a plain file needs sealing");
    let config = load(&folder.config()).0;
    save(&folder.config(), &config).unwrap();

    assert!(!needs_sealing(&folder.config()), "a sealed file doesn't need sealing again");
    let bytes = std::fs::read(folder.config()).unwrap();
    assert!(bytes.starts_with(br"SMONCFIG"), "the rewrite is a seal");
    assert!(!String::from_utf8_lossy(&bytes).contains("sk-strata-secret"), "and the key is sealed away");
    assert_eq!(load(&folder.config()).0.servers[0].key, "sk-strata-secret", "reading it back still works");
}

#[cfg(not(windows))]
#[test]
fn a_plain_file_stays_plain_where_there_is_no_seal_to_apply() {
    let folder = Folder::new();
    let json = r#"{ "servers": [{ "name": "NEDM", "addresses": ["192.168.1.5"], "key": "sk-strata-secret" }] }"#;
    std::fs::write(folder.config(), json).unwrap();

    assert!(!needs_sealing(&folder.config()));
    let config = load(&folder.config()).0;
    save(&folder.config(), &config).unwrap();

    let after = std::fs::read_to_string(folder.config()).unwrap();
    assert!(after.starts_with('{'), "the rewrite is still plain JSON, not a seal");
    assert!(after.contains("sk-strata-secret"), "and the key stays plain where there's no seal to apply");
    assert_eq!(load(&folder.config()).0.servers[0].key, "sk-strata-secret");
}

#[test]
fn saving_creates_the_folder_and_leaves_no_temporary_file() {
    let folder = Folder::new();
    let path = folder.0.join("new").join("config.json");
    save(&path, &AppConfig::default()).unwrap();

    assert!(path.exists());
    assert_eq!(std::fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
}

#[test]
fn settings_live_under_the_new_name_and_are_looked_for_under_the_old_one() {
    assert!(default_path().ends_with("StrataMon/config.json"));
    assert!(old_default_path().ends_with("StrataWatch/config.json"));
    assert_eq!(default_path().parent().unwrap().parent(), old_default_path().parent().unwrap().parent());
}

const OLD_SETTINGS: &str = r#"{ "servers": [{ "name": "NEDM", "addresses": ["192.168.1.5"], "key": "secret" }], "someFutureSetting": 1 }"#;

fn old_and_new(folder: &Folder) -> (PathBuf, PathBuf) {
    let old = folder.0.join("StrataWatch").join("config.json");
    std::fs::create_dir_all(old.parent().unwrap()).unwrap();
    std::fs::write(&old, OLD_SETTINGS).unwrap();
    (old, folder.0.join("StrataMon").join("config.json"))
}

#[test]
fn settings_under_the_old_name_are_copied_with_their_keys_and_left_in_place() {
    let folder = Folder::new();
    let (old, new) = old_and_new(&folder);

    adopt_old_settings(&old, &new).unwrap();

    assert_eq!(load(&new).0.servers[0].key, "secret");
    assert_eq!(std::fs::read_to_string(&new).unwrap(), OLD_SETTINGS, "copied exactly, including settings this build doesn't know");
    assert_eq!(std::fs::read_to_string(&old).unwrap(), OLD_SETTINGS);
    assert_eq!(std::fs::read_dir(new.parent().unwrap()).unwrap().count(), 1, "no temporary file left");
}

#[test]
fn settings_under_the_new_name_are_never_replaced_by_old_ones() {
    let folder = Folder::new();
    let (old, new) = old_and_new(&folder);
    std::fs::create_dir_all(new.parent().unwrap()).unwrap();
    std::fs::write(&new, "{}").unwrap();

    adopt_old_settings(&old, &new).unwrap();

    assert_eq!(std::fs::read_to_string(&new).unwrap(), "{}");
}

#[test]
fn with_no_old_settings_nothing_is_created() {
    let folder = Folder::new();
    let new = folder.0.join("StrataMon").join("config.json");

    adopt_old_settings(&folder.0.join("StrataWatch").join("config.json"), &new).unwrap();

    assert!(!new.parent().unwrap().exists());
}

#[test]
fn a_failed_copy_is_reported_and_keeps_the_old_settings() {
    let folder = Folder::new();
    let (old, _) = old_and_new(&folder);
    // A file where the folder should be, which is the same failure as a folder that can't be made.
    let blocker = folder.0.join("StrataMon");
    std::fs::write(&blocker, "in the way").unwrap();

    assert!(adopt_old_settings(&old, &blocker.join("config.json")).is_err());

    assert_eq!(std::fs::read_to_string(&old).unwrap(), OLD_SETTINGS);
    assert_eq!(std::fs::read_to_string(&blocker).unwrap(), "in the way");
}
