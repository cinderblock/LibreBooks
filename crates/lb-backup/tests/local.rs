//! Backups through a local folder: round trips with and without encryption,
//! tampering, retention, the portable file and the scheduler.

mod common;

use std::sync::Arc;
use std::time::Duration;

use age::secrecy::{ExposeSecret, SecretString};
use jiff::SignedDuration;
use lb_backup::{
    BackupConfig, BackupError, Destination, Encryption, EncryptionKind, EnvAndFiles, Retention,
    RetentionScope, Schedule, Scheduler, SecretRef, SecretResolver, Unlock, backup_now, export_lbk,
    import_lbk, list_backups, restore, verify,
};
use lb_store::Books;

/// Resolves `test:` references from a fixed table, like the server's store would.
struct Fixed(&'static str, &'static str);

impl SecretResolver for Fixed {
    fn resolve(&self, r: &SecretRef) -> lb_backup::Result<SecretString> {
        if r.0 == self.0 {
            Ok(SecretString::from(self.1.to_owned()))
        } else {
            EnvAndFiles.resolve(r)
        }
    }
}

fn local(dir: &std::path::Path) -> BackupConfig {
    BackupConfig {
        destinations: vec![Destination::Local {
            dir: dir.join("backups"),
        }],
        time_zone: Some("UTC".into()),
        ..BackupConfig::default()
    }
}

#[tokio::test]
async fn an_unencrypted_backup_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let (books, _) = common::company(dir.path(), "Acme Widgets");
    let config = local(dir.path());

    let record = backup_now(&books, "Acme Widgets", &config, &EnvAndFiles)
        .await
        .unwrap();
    assert!(record.complete());
    let m = &record.manifest;
    assert_eq!(m.company_slug, "acme-widgets");
    assert_eq!(m.encryption, EncryptionKind::None);
    assert!(m.schema_version > 0);
    assert!(m.stored_size < m.database_size, "zstd made it smaller");
    assert!(m.artifact.ends_with(".lbdb.zst"));
    assert!(dir.path().join("backups").join(&m.artifact).exists());

    // No working files left beside the company file.
    let stray: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with(".lb-"))
        .collect();
    assert!(stray.is_empty(), "{stray:?}");

    let list = list_backups(&config.destinations[0], &EnvAndFiles, Some("acme-widgets"))
        .await
        .unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(&list[0].manifest, m);

    let report = verify(&list[0], &EnvAndFiles, &Unlock::None).await.unwrap();
    assert!(report.ok() && report.contents_checked, "{report:?}");

    let dest = dir.path().join("restored").join("acme.lbdb");
    let restored = restore(&list[0], &dest, &EnvAndFiles, &Unlock::None)
        .await
        .unwrap();
    assert_eq!(&restored, m);
    assert_eq!(common::account_names(&dest), ["Checking", "Rent"]);
    let reopened = Books::open(&dest).unwrap();
    assert_eq!(reopened.check_integrity().unwrap(), Vec::<String>::new());
    drop(reopened);

    // Never over an existing file.
    let again = restore(&list[0], &dest, &EnvAndFiles, &Unlock::None).await;
    assert!(matches!(again, Err(BackupError::Exists(_))), "{again:?}");
}

#[tokio::test]
async fn a_backup_encrypted_to_a_key_needs_the_identity() {
    let dir = tempfile::tempdir().unwrap();
    let (books, _) = common::company(dir.path(), "Acme");
    let identity = age::x25519::Identity::generate();
    let other = age::x25519::Identity::generate();
    let mut config = local(dir.path());
    config.encryption = Encryption::Recipients {
        recipients: vec![identity.to_public().to_string()],
    };

    let record = backup_now(&books, "Acme", &config, &EnvAndFiles)
        .await
        .unwrap();
    assert_eq!(record.manifest.encryption, EncryptionKind::X25519);
    assert!(record.manifest.artifact.ends_with(".lbdb.zst.age"));
    let entry = &list_backups(&config.destinations[0], &EnvAndFiles, None)
        .await
        .unwrap()[0];

    // Without the key, only the checksum can be checked.
    let report = verify(entry, &EnvAndFiles, &Unlock::None).await.unwrap();
    assert!(report.checksum_ok && !report.contents_checked && report.ok());

    let dest = dir.path().join("r.lbdb");
    let no_key = restore(entry, &dest, &EnvAndFiles, &Unlock::None).await;
    assert!(
        matches!(&no_key, Err(BackupError::Decrypt(m)) if m.contains("AGE-SECRET-KEY")),
        "{no_key:?}"
    );
    let wrong = Unlock::Identities(vec![other.to_string().expose_secret().to_owned()]);
    let wrong_key = restore(entry, &dest, &EnvAndFiles, &wrong).await;
    assert!(matches!(wrong_key, Err(BackupError::Decrypt(_))));
    assert!(!dest.exists(), "nothing is left behind by a failed restore");

    let right = Unlock::Identities(vec![identity.to_string().expose_secret().to_owned()]);
    let report = verify(entry, &EnvAndFiles, &right).await.unwrap();
    assert!(report.ok() && report.contents_checked, "{report:?}");
    restore(entry, &dest, &EnvAndFiles, &right).await.unwrap();
    assert_eq!(common::account_names(&dest), ["Checking", "Rent"]);
}

#[tokio::test]
async fn a_passphrase_backup_can_be_checked_by_the_server_itself() {
    let dir = tempfile::tempdir().unwrap();
    let (books, _) = common::company(dir.path(), "Acme");
    let mut config = local(dir.path());
    config.encryption = Encryption::Passphrase {
        passphrase_ref: SecretRef::new("test:pp"),
        // Cheap scrypt for tests; real backups leave this to age.
        work_factor: Some(10),
    };
    let secrets = Fixed("test:pp", "correct horse battery staple");

    let record = backup_now(&books, "Acme", &config, &secrets).await.unwrap();
    assert_eq!(record.manifest.encryption, EncryptionKind::Passphrase);
    let entry = &list_backups(&config.destinations[0], &secrets, None)
        .await
        .unwrap()[0];

    let unlock = Unlock::from_config(&config.encryption, &secrets).unwrap();
    let report = verify(entry, &secrets, &unlock).await.unwrap();
    assert!(report.ok() && report.contents_checked, "{report:?}");

    let wrong = Unlock::Passphrase(SecretString::from("Tr0ub4dor&3".to_owned()));
    let dest = dir.path().join("r.lbdb");
    let err = restore(entry, &dest, &secrets, &wrong).await.unwrap_err();
    assert!(matches!(err, BackupError::Decrypt(_)), "{err}");
    restore(entry, &dest, &secrets, &unlock).await.unwrap();
    assert_eq!(common::account_names(&dest), ["Checking", "Rent"]);
}

#[tokio::test]
async fn tampering_is_detected() {
    let dir = tempfile::tempdir().unwrap();
    let (books, _) = common::company(dir.path(), "Acme");
    let config = local(dir.path());
    let record = backup_now(&books, "Acme", &config, &EnvAndFiles)
        .await
        .unwrap();
    let artifact = dir.path().join("backups").join(&record.manifest.artifact);
    let entry = &list_backups(&config.destinations[0], &EnvAndFiles, None)
        .await
        .unwrap()[0];

    // One flipped bit in the stored bytes.
    let mut bytes = std::fs::read(&artifact).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0x10;
    std::fs::write(&artifact, &bytes).unwrap();

    let report = verify(entry, &EnvAndFiles, &Unlock::None).await.unwrap();
    assert!(!report.ok() && !report.checksum_ok, "{report:?}");
    assert!(report.problems[0].contains("checksum"));
    let dest = dir.path().join("r.lbdb");
    let err = restore(entry, &dest, &EnvAndFiles, &Unlock::None)
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            BackupError::Checksum {
                what: "the stored backup"
            }
        ),
        "{err}"
    );
    assert!(!dest.exists());

    // A manifest that lies about the database inside is caught after unpacking.
    bytes[mid] ^= 0x10;
    std::fs::write(&artifact, &bytes).unwrap();
    let mut lying = entry.clone();
    lying.manifest.database_sha256 = "00".repeat(32);
    let report = verify(&lying, &EnvAndFiles, &Unlock::None).await.unwrap();
    assert!(report.checksum_ok && !report.ok(), "{report:?}");
    let err = restore(&lying, &dest, &EnvAndFiles, &Unlock::None)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        BackupError::Checksum {
            what: "the restored database"
        }
    ));
    assert!(!dest.exists());
}

#[tokio::test]
async fn retention_prunes_after_each_backup() {
    let dir = tempfile::tempdir().unwrap();
    let (books, _) = common::company(dir.path(), "Acme");
    let mut config = local(dir.path());
    config.retention = Retention {
        keep_last: 2,
        daily: 0,
        weekly: 0,
        monthly: 0,
        yearly: 0,
    };
    let mut ids = Vec::new();
    for _ in 0..4 {
        let r = backup_now(&books, "Acme", &config, &EnvAndFiles)
            .await
            .unwrap();
        ids.push(r.manifest.id.clone());
        // Distinct millisecond stamps.
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let left: Vec<String> = list_backups(&config.destinations[0], &EnvAndFiles, None)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.manifest.id)
        .collect();
    assert_eq!(left, [ids[3].clone(), ids[2].clone()]);
    // Their artifacts went too.
    let files = std::fs::read_dir(dir.path().join("backups").join("acme"))
        .unwrap()
        .count();
    assert_eq!(files, 4, "two backups, two files each");

    // Another company in the same folder is not touched by Acme's retention.
    let (other, _) = common::company(dir.path(), "Other Co");
    backup_now(&other, "Other Co", &config, &EnvAndFiles)
        .await
        .unwrap();
    backup_now(&books, "Acme", &config, &EnvAndFiles)
        .await
        .unwrap();
    let all = list_backups(&config.destinations[0], &EnvAndFiles, None)
        .await
        .unwrap();
    assert_eq!(
        all.iter()
            .filter(|e| e.manifest.company_slug == "other-co")
            .count(),
        1
    );
}

#[tokio::test]
async fn one_failing_destination_does_not_fail_the_backup() {
    let dir = tempfile::tempdir().unwrap();
    let (books, _) = common::company(dir.path(), "Acme");
    let blocker = dir.path().join("a-file");
    std::fs::write(&blocker, b"x").unwrap();
    let mut config = local(dir.path());
    config.destinations.push(Destination::Local {
        dir: blocker.join("cannot-be-a-folder"),
    });
    let record = backup_now(&books, "Acme", &config, &EnvAndFiles)
        .await
        .unwrap();
    assert!(!record.complete());
    assert!(record.destinations[0].stored);
    assert!(!record.destinations[1].stored);
    assert!(record.destinations[1].error.is_some());

    config.destinations.remove(0);
    let err = backup_now(&books, "Acme", &config, &EnvAndFiles)
        .await
        .unwrap_err();
    assert!(
        matches!(err, BackupError::AllDestinationsFailed(_)),
        "{err}"
    );
    config.destinations.clear();
    let err = backup_now(&books, "Acme", &config, &EnvAndFiles)
        .await
        .unwrap_err();
    assert!(matches!(err, BackupError::NoDestinations));
}

#[test]
fn the_portable_file_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let (books, _) = common::company(dir.path(), "Acme");
    let lbk = dir.path().join("acme.lbk");
    let m = export_lbk(&books, "Acme", &lbk).unwrap();
    assert_eq!(m.company_name, "Acme");
    assert!(matches!(
        export_lbk(&books, "Acme", &lbk),
        Err(BackupError::Exists(_))
    ));

    // An ordinary zip, readable by anything.
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&lbk).unwrap()).unwrap();
    let mut names: Vec<String> = zip.file_names().map(|n| n.unwrap().into_owned()).collect();
    names.sort_unstable();
    assert_eq!(names, ["company.lbdb", "manifest.json"]);
    assert_eq!(
        zip.by_name("company.lbdb").unwrap().compression(),
        zip::CompressionMethod::Deflated
    );

    let dest = dir.path().join("moved").join("acme.lbdb");
    let back = import_lbk(&lbk, &dest).unwrap();
    assert_eq!(back, m);
    assert_eq!(common::account_names(&dest), ["Checking", "Rent"]);
    assert!(matches!(
        import_lbk(&lbk, &dest),
        Err(BackupError::Exists(_))
    ));
}

#[test]
fn a_damaged_or_foreign_portable_file_is_refused() {
    use std::io::Write as _;
    let dir = tempfile::tempdir().unwrap();
    let (books, _) = common::company(dir.path(), "Acme");
    let lbk = dir.path().join("acme.lbk");
    export_lbk(&books, "Acme", &lbk).unwrap();

    // Same zip, with the database swapped for other bytes.
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&lbk).unwrap()).unwrap();
    let mut manifest = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("manifest.json").unwrap(), &mut manifest)
        .unwrap();
    let forged = dir.path().join("forged.lbk");
    let mut w = zip::ZipWriter::new(std::fs::File::create(&forged).unwrap());
    let o = zip::write::SimpleFileOptions::default();
    w.start_file("manifest.json", o).unwrap();
    w.write_all(manifest.as_bytes()).unwrap();
    w.start_file("company.lbdb", o).unwrap();
    w.write_all(b"definitely not a database").unwrap();
    w.finish().unwrap();
    let dest = dir.path().join("x.lbdb");
    let err = import_lbk(&forged, &dest).unwrap_err();
    assert!(matches!(err, BackupError::Checksum { .. }), "{err}");
    assert!(!dest.exists());

    // A zip that is not ours at all.
    let other = dir.path().join("other.zip");
    let mut w = zip::ZipWriter::new(std::fs::File::create(&other).unwrap());
    w.start_file("manifest.json", o).unwrap();
    w.write_all(br#"{"kind":"something-else","format":1}"#)
        .unwrap();
    w.finish().unwrap();
    let err = import_lbk(&other, &dest).unwrap_err();
    assert!(
        err.to_string().contains("not a LibreBooks company file"),
        "{err}"
    );
    std::fs::write(dir.path().join("plain.txt"), "hello").unwrap();
    assert!(import_lbk(&dir.path().join("plain.txt"), &dest).is_err());
}

#[tokio::test]
async fn the_scheduler_backs_up_when_due_and_on_request() {
    let dir = tempfile::tempdir().unwrap();
    let (books, _) = common::company(dir.path(), "Acme");
    let mut config = local(dir.path());
    config.schedule = Schedule {
        enabled: true,
        every: Some(SignedDuration::from_hours(6)),
        daily_at: None,
    };
    let scheduler = Scheduler::start(books, "Acme", config.clone(), Arc::new(EnvAndFiles));

    // Never backed up: due at once.
    let first = wait_for(&scheduler, 1).await;
    let made = first.last.as_ref().unwrap().record.as_ref().unwrap();
    let next = first.next_run.unwrap();
    assert_eq!(
        next,
        made.manifest
            .created_at
            .checked_add(SignedDuration::from_hours(6))
            .unwrap()
    );

    scheduler.run_now();
    let second = wait_for(&scheduler, 2).await;
    assert!(second.last.unwrap().error.is_none());

    // Off: nothing scheduled, but a manual run still works.
    config.schedule.enabled = false;
    scheduler.update(config.clone());
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(scheduler.status().next_run, None);
    scheduler.shutdown().await;

    let n = list_backups(&config.destinations[0], &EnvAndFiles, None)
        .await
        .unwrap()
        .len();
    assert_eq!(n, 2);
}

#[tokio::test]
async fn a_restarted_scheduler_remembers_the_last_backup() {
    let dir = tempfile::tempdir().unwrap();
    let (books, _) = common::company(dir.path(), "Acme");
    let config = local(dir.path());
    let made = backup_now(&books, "Acme", &config, &EnvAndFiles)
        .await
        .unwrap();
    let scheduler = Scheduler::start(books, "Acme", config.clone(), Arc::new(EnvAndFiles));
    let mut status = scheduler.status();
    for _ in 0..100 {
        if status.next_run.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        status = scheduler.status();
    }
    assert_eq!(status.last_success_at, Some(made.manifest.created_at));
    assert_eq!(
        status.next_run,
        made.manifest
            .created_at
            .checked_add(SignedDuration::from_hours(24))
            .ok()
    );
    assert!(status.last.is_none(), "nothing was due");
    scheduler.shutdown().await;
}

/// Wait until the scheduler has finished `n` backups (successful ones).
async fn wait_for(s: &Scheduler, n: usize) -> lb_backup::SchedulerStatus {
    let start = std::time::Instant::now();
    let mut seen = std::collections::HashSet::new();
    loop {
        let st = s.status();
        if let Some(r) = st.last.as_ref().and_then(|l| l.record.as_ref()) {
            seen.insert(r.manifest.id.clone());
        }
        let rescheduled = st
            .last
            .as_ref()
            .and_then(|l| l.record.as_ref())
            .is_some_and(|r| st.next_run.is_some_and(|t| t > r.manifest.created_at));
        if seen.len() >= n && !st.running && rescheduled {
            return st;
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "scheduler stalled: {st:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// A renamed company's backups sit under its old slug. In a destination of its
/// own (the server's per-company folders) retention covers them too; in a
/// shared one it stays within the current slug.
#[tokio::test]
async fn a_renamed_companys_old_backups_age_out_in_its_own_destination() {
    let dir = tempfile::tempdir().unwrap();
    let (books, _) = common::company(dir.path(), "Acme");
    let mut config = local(dir.path());
    config.retention = Retention {
        keep_last: 1,
        daily: 0,
        weekly: 0,
        monthly: 0,
        yearly: 0,
    };
    backup_now(&books, "Acme", &config, &EnvAndFiles)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(5)).await;

    // Shared destination: the rename starts a new folder and the old one stays.
    let r = backup_now(&books, "Acme Widgets", &config, &EnvAndFiles)
        .await
        .unwrap();
    assert_eq!(r.destinations[0].pruned, Vec::<String>::new());
    let slugs = |entries: Vec<lb_backup::BackupEntry>| -> Vec<String> {
        let mut v: Vec<String> = entries
            .into_iter()
            .map(|e| e.manifest.company_slug)
            .collect();
        v.sort();
        v
    };
    let all = list_backups(&config.destinations[0], &EnvAndFiles, None)
        .await
        .unwrap();
    assert_eq!(slugs(all), ["acme", "acme-widgets"]);
    tokio::time::sleep(Duration::from_millis(5)).await;

    // A destination of its own: everything in it is this company's.
    config.retention_scope = RetentionScope::Destination;
    let r = backup_now(&books, "Acme Widgets", &config, &EnvAndFiles)
        .await
        .unwrap();
    assert_eq!(
        r.destinations[0].pruned.len(),
        2,
        "{:?}",
        r.destinations[0].pruned
    );
    let all = list_backups(&config.destinations[0], &EnvAndFiles, None)
        .await
        .unwrap();
    assert_eq!(slugs(all), ["acme-widgets"]);
    assert!(
        !dir.path().join("backups").join("acme").exists()
            || std::fs::read_dir(dir.path().join("backups").join("acme"))
                .unwrap()
                .count()
                == 0,
        "the old slug's folder is empty"
    );
}
