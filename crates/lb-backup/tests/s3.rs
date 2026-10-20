//! Backups through an S3-compatible bucket, against a local look-alike that checks
//! every request's signature.

mod common;

use age::secrecy::SecretString;
use lb_backup::dest::Store;
use lb_backup::{
    BackupConfig, BackupError, Destination, EnvAndFiles, Retention, SecretRef, SecretResolver,
    Unlock, backup_now, list_backups, restore, verify,
};

struct Keys(&'static str);

impl SecretResolver for Keys {
    fn resolve(&self, r: &SecretRef) -> lb_backup::Result<SecretString> {
        match r.0.as_str() {
            "test:s3" => Ok(SecretString::from(self.0.to_owned())),
            _ => EnvAndFiles.resolve(r),
        }
    }
}

fn s3(endpoint: &str, prefix: &str) -> Destination {
    Destination::S3 {
        endpoint: endpoint.into(),
        region: "us-east-1".into(),
        bucket: common::BUCKET.into(),
        prefix: prefix.into(),
        access_key_id: common::ACCESS_KEY.into(),
        secret_access_key_ref: SecretRef::new("test:s3"),
        path_style: true,
    }
}

#[tokio::test]
async fn put_get_list_delete_through_the_store() {
    let (endpoint, mock) = common::start_mock_s3().await;
    let store = Store::open(&s3(&endpoint, "lb"), &Keys(common::SECRET_KEY)).unwrap();

    // Names that need encoding are signed and sent the same way.
    for name in ["a/one", "a/two words", "a/three+$&=", "b/four"] {
        store
            .put_bytes(name, name.as_bytes().to_vec())
            .await
            .unwrap();
    }
    assert!(mock.objects.lock().unwrap().contains_key("lb/a/two words"));
    // Four keys with a page size of three: listing follows the continuation.
    assert_eq!(
        store.list("").await.unwrap(),
        ["a/one", "a/three+$&=", "a/two words", "b/four"]
    );
    assert_eq!(store.list("a/").await.unwrap().len(), 3);
    assert_eq!(
        store.get_bytes("a/three+$&=").await.unwrap(),
        b"a/three+$&="
    );

    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("f");
    store.get_file("a/one", &file).await.unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), b"a/one");

    store.delete("a/one").await.unwrap();
    store.delete("a/one").await.unwrap();
    assert!(matches!(
        store.get_bytes("a/one").await,
        Err(BackupError::NotFound(_))
    ));
    store.check().await.unwrap();
    assert_eq!(
        store.list("").await.unwrap().len(),
        3,
        "the probe cleaned up"
    );
}

#[tokio::test]
async fn the_services_errors_come_back_in_words() {
    let (endpoint, _mock) = common::start_mock_s3().await;
    let store = Store::open(&s3(&endpoint, ""), &Keys("the-wrong-secret")).unwrap();
    let err = store.put_bytes("x", b"x".to_vec()).await.unwrap_err();
    assert!(
        matches!(&err, BackupError::Remote { status: 403, message } if message.starts_with("SignatureDoesNotMatch")),
        "{err}"
    );
    let unreachable = Store::open(&s3("http://127.0.0.1:1", ""), &Keys("k")).unwrap();
    let err = unreachable.list("").await.unwrap_err();
    assert!(matches!(err, BackupError::Network(_)), "{err}");
}

#[tokio::test]
async fn a_backup_round_trips_through_a_bucket() {
    let (endpoint, mock) = common::start_mock_s3().await;
    let dir = tempfile::tempdir().unwrap();
    let (books, _) = common::company(dir.path(), "Acme");
    let secrets = Keys(common::SECRET_KEY);
    let config = BackupConfig {
        destinations: vec![s3(&endpoint, "librebooks/")],
        retention: Retention {
            keep_last: 1,
            daily: 0,
            weekly: 0,
            monthly: 0,
            yearly: 0,
        },
        ..BackupConfig::default()
    };

    let first = backup_now(&books, "Acme", &config, &secrets).await.unwrap();
    assert!(first.complete(), "{first:?}");
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let second = backup_now(&books, "Acme", &config, &secrets).await.unwrap();
    assert_eq!(
        second.destinations[0].pruned,
        std::slice::from_ref(&first.manifest.id)
    );

    let keys: Vec<String> = mock.objects.lock().unwrap().keys().cloned().collect();
    assert_eq!(
        keys,
        [
            format!("librebooks/{}", second.manifest.artifact),
            format!("librebooks/{}", second.manifest.object_name()),
        ]
    );

    let list = list_backups(&config.destinations[0], &secrets, Some("acme"))
        .await
        .unwrap();
    assert_eq!(list.len(), 1);
    let report = verify(&list[0], &secrets, &Unlock::None).await.unwrap();
    assert!(report.ok() && report.contents_checked, "{report:?}");
    let dest = dir.path().join("restored.lbdb");
    restore(&list[0], &dest, &secrets, &Unlock::None)
        .await
        .unwrap();
    assert_eq!(common::account_names(&dest), ["Checking", "Rent"]);

    // Bytes changed in the bucket are caught.
    let artifact = format!("librebooks/{}", second.manifest.artifact);
    mock.objects
        .lock()
        .unwrap()
        .get_mut(&artifact)
        .unwrap()
        .push(0);
    let report = verify(&list[0], &secrets, &Unlock::None).await.unwrap();
    assert!(!report.checksum_ok);
}
