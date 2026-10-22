//! `xremail-api` 的 T4 测试：L2 契约。
//!
//! 覆盖 [ARCHITECTURE §8](../../../docs/ARCHITECTURE.md) 明确要求的四项：
//! 首次写、往返、双设备 CAS 竞争、错误返回。
//!
//! 另加一条关键性质：**契约里不提供"先查后写"的组合**。
//! 若将来有人加了 `check_conflict` + `push` 这样的组合方法，
//! 就会引入 TOCTOU。本测试通过"只用 CAS 也能完成一切"来固定这个设计。

use xremail_api::mock::{InMemorySecretStore, InMemorySyncBackend, InMemoryVaultStore};
use xremail_api::{
    RemoteEntry, Revision, SecretStore, SecretValue, SyncBackend, SyncError, VaultEntry, VaultStore,
};

// ===========================================================================
// SecretStore
// ===========================================================================

#[tokio::test]
async fn secret_store_round_trip() {
    let store = InMemorySecretStore::new();

    store
        .set(
            "keychain::a1",
            &SecretValue::from_str_lossless("授权码-123"),
        )
        .await
        .unwrap();

    let got = store.get("keychain::a1").await.unwrap();
    assert_eq!(got.expect("应存在").as_bytes(), "授权码-123".as_bytes());
}

#[tokio::test]
async fn secret_store_missing_entry_is_none_not_error() {
    let store = InMemorySecretStore::new();

    let got = store.get("keychain::nope").await.unwrap();
    assert!(got.is_none(), "“没有”不是错误");
}

#[tokio::test]
async fn secret_store_delete_reports_whether_it_removed_something() {
    let store = InMemorySecretStore::new();
    store
        .set("keychain::a1", &SecretValue::new(vec![1, 2, 3]))
        .await
        .unwrap();

    assert!(store.delete("keychain::a1").await.unwrap());
    assert!(
        !store.delete("keychain::a1").await.unwrap(),
        "重复删除应返回 false"
    );
    assert!(store.get("keychain::a1").await.unwrap().is_none());
}

#[tokio::test]
async fn secret_store_overwrites_existing_value() {
    let store = InMemorySecretStore::new();
    store
        .set("keychain::a1", &SecretValue::new(vec![1]))
        .await
        .unwrap();
    store
        .set("keychain::a1", &SecretValue::new(vec![2]))
        .await
        .unwrap();

    assert_eq!(store.len(), 1);
    assert_eq!(
        store.get("keychain::a1").await.unwrap().unwrap().as_bytes(),
        &[2]
    );
}

#[tokio::test]
async fn secret_value_debug_does_not_leak_content() {
    let value = SecretValue::from_str_lossless("super-secret-token");

    let printed = format!("{value:?}");

    assert!(
        !printed.contains("super-secret-token"),
        "机密泄漏到了 Debug 输出"
    );
    assert!(printed.contains("len"));
}

#[tokio::test]
async fn secret_store_rejects_empty_reference() {
    let store = InMemorySecretStore::new();

    let err = store.set("", &SecretValue::new(vec![1])).await.unwrap_err();

    assert!(!err.is_retryable());
}

// ===========================================================================
// VaultStore
// ===========================================================================

#[tokio::test]
async fn vault_store_round_trip() {
    let store = InMemoryVaultStore::new();
    let entry =
        VaultEntry::new("v1", "credential", vec![9, 9, 9], 1_700_000_000_000).with_account("a1");

    store.put(&entry).await.unwrap();

    let got = store.get("v1").await.unwrap().expect("应存在");
    assert_eq!(got, entry);
    assert_eq!(got.account_id.as_deref(), Some("a1"));
    assert_eq!(got.version, 1);
}

#[tokio::test]
async fn vault_store_missing_entry_is_none_not_error() {
    let store = InMemoryVaultStore::new();

    assert!(store.get("nope").await.unwrap().is_none());
}

#[tokio::test]
async fn vault_store_list_is_stable_and_sorted() {
    let store = InMemoryVaultStore::new();
    for id in ["v3", "v1", "v2"] {
        store
            .put(&VaultEntry::new(id, "credential", vec![1], 0))
            .await
            .unwrap();
    }

    let ids: Vec<String> = store
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.id)
        .collect();

    assert_eq!(ids, vec!["v1", "v2", "v3"]);
}

#[tokio::test]
async fn vault_store_delete_reports_whether_it_removed_something() {
    let store = InMemoryVaultStore::new();
    store
        .put(&VaultEntry::new("v1", "credential", vec![1], 0))
        .await
        .unwrap();

    assert!(store.delete("v1").await.unwrap());
    assert!(!store.delete("v1").await.unwrap());
}

#[tokio::test]
async fn vault_store_rejects_empty_envelope() {
    let store = InMemoryVaultStore::new();

    let err = store
        .put(&VaultEntry::new("v1", "credential", Vec::new(), 0))
        .await
        .unwrap_err();

    assert!(!err.is_retryable(), "内容不合法，重试没有意义");
}

// ===========================================================================
// SyncBackend：首次写
// ===========================================================================

#[tokio::test]
async fn first_write_succeeds_when_entry_does_not_exist() {
    let backend = InMemorySyncBackend::new();

    let revision = backend
        .compare_and_swap("account/a1", None, b"payload-v1")
        .await
        .unwrap();

    assert_eq!(revision, Revision(1));
    let entry = backend.read("account/a1").await.unwrap().expect("应存在");
    assert_eq!(entry.payload, b"payload-v1");
    assert_eq!(entry.revision, Revision(1));
}

#[tokio::test]
async fn first_write_conflicts_when_entry_already_exists() {
    let backend = InMemorySyncBackend::new();
    backend
        .compare_and_swap("account/a1", None, b"payload-v1")
        .await
        .unwrap();

    // expected = None 要求"远端尚无该条目"，现在有了 → 冲突。
    let err = backend
        .compare_and_swap("account/a1", None, b"payload-v2")
        .await
        .unwrap_err();

    assert!(err.is_conflict());
    match err {
        SyncError::Conflict { current } => assert_eq!(current, Some(Revision(1))),
        other => panic!("期望冲突，实际是 {other:?}"),
    }
}

// ===========================================================================
// SyncBackend：往返
// ===========================================================================

#[tokio::test]
async fn read_write_read_round_trip() {
    let backend = InMemorySyncBackend::new();

    let rev1 = backend.compare_and_swap("k", None, b"v1").await.unwrap();
    let entry = backend.read("k").await.unwrap().unwrap();
    assert_eq!(entry.payload, b"v1");

    let rev2 = backend
        .compare_and_swap("k", Some(rev1), b"v2")
        .await
        .unwrap();
    assert!(rev2 > rev1, "修订号必须单调递增");

    let entry = backend.read("k").await.unwrap().unwrap();
    assert_eq!(entry.payload, b"v2");
    assert_eq!(entry.revision, rev2);
}

#[tokio::test]
async fn reading_missing_entry_is_none_not_error() {
    let backend = InMemorySyncBackend::new();

    assert!(backend.read("nope").await.unwrap().is_none());
}

// ===========================================================================
// SyncBackend：双设备 CAS 竞争（核心）
// ===========================================================================

#[tokio::test]
async fn two_devices_racing_produce_exactly_one_winner() {
    // 两个"设备"共享同一个远端。
    let backend = InMemorySyncBackend::new();

    // 双方都读到 rev = 1。
    backend.compare_and_swap("k", None, b"base").await.unwrap();
    let device_a = backend.read("k").await.unwrap().unwrap();
    let device_b = backend.read("k").await.unwrap().unwrap();
    assert_eq!(device_a.revision, device_b.revision);

    // A 提交成功。
    let a = backend
        .compare_and_swap("k", Some(device_a.revision), b"from-a")
        .await;

    // B 用**过期的**修订号提交 → 必须冲突。
    let b = backend
        .compare_and_swap("k", Some(device_b.revision), b"from-b")
        .await;

    assert!(a.is_ok(), "A 应当成功");
    let b_err = b.expect_err("B 必须冲突");
    assert!(b_err.is_conflict());

    // 远端内容是赢家的，不是混合的。
    let final_entry = backend.read("k").await.unwrap().unwrap();
    assert_eq!(final_entry.payload, b"from-a", "远端应是 A 的内容");
    assert_eq!(final_entry.revision, Revision(2));
}

#[tokio::test]
async fn conflicting_write_reports_the_current_remote_revision() {
    let backend = InMemorySyncBackend::new();
    backend.compare_and_swap("k", None, b"base").await.unwrap();

    let stale = backend.read("k").await.unwrap().unwrap().revision;
    backend
        .compare_and_swap("k", Some(stale), b"from-a")
        .await
        .unwrap();

    let err = backend
        .compare_and_swap("k", Some(stale), b"from-b")
        .await
        .unwrap_err();

    // 冲突错误必须带上远端当前的修订号，调用方据此重新读取并合并。
    match err {
        SyncError::Conflict { current } => assert_eq!(current, Some(Revision(2))),
        other => panic!("期望冲突，实际是 {other:?}"),
    }
}

#[tokio::test]
async fn stale_expected_revision_on_missing_entry_is_a_conflict() {
    let backend = InMemorySyncBackend::new();

    // 远端没有该条目，却提交 expected = Some(rev) → 冲突（current = None）。
    let err = backend
        .compare_and_swap("k", Some(Revision(5)), b"payload")
        .await
        .unwrap_err();

    match err {
        SyncError::Conflict { current } => assert_eq!(current, None),
        other => panic!("期望冲突，实际是 {other:?}"),
    }
}

#[tokio::test]
async fn conflict_is_not_retryable() {
    let backend = InMemorySyncBackend::new();
    backend.compare_and_swap("k", None, b"base").await.unwrap();
    let rev = backend.read("k").await.unwrap().unwrap().revision;
    backend
        .compare_and_swap("k", Some(rev), b"a")
        .await
        .unwrap();

    let err = backend
        .compare_and_swap("k", Some(rev), b"b")
        .await
        .unwrap_err();

    assert!(
        !err.is_retryable(),
        "重试同一份提交只会再次冲突，必须重新读取并合并"
    );
}

#[tokio::test]
async fn three_way_race_still_has_exactly_one_winner() {
    let backend = InMemorySyncBackend::new();
    backend.compare_and_swap("k", None, b"base").await.unwrap();

    let rev = backend.read("k").await.unwrap().unwrap().revision;

    let results = [
        backend.compare_and_swap("k", Some(rev), b"a").await,
        backend.compare_and_swap("k", Some(rev), b"b").await,
        backend.compare_and_swap("k", Some(rev), b"c").await,
    ];

    let winners = results.iter().filter(|r| r.is_ok()).count();
    let conflicts = results.iter().filter(|r| r.is_err()).count();

    assert_eq!(winners, 1, "三方竞争必须只有一个赢家");
    assert_eq!(conflicts, 2);
}

#[tokio::test]
async fn api_shape_alone_prevents_toctou_even_for_a_naive_caller() {
    // 即便调用方自己做了「先读、看看冲不冲突、再写」，
    // 只要写入走的是条件提交，就不会丢更新。
    //
    // 原因：本地那次「看看冲不冲突」只是**本地判断**，
    // 真正拦住丢失更新的是 compare_and_swap 的原子条件检查。
    // 若契约提供的是「无条件推送」，同样的代码就会静默覆盖别人的写入。
    let backend = InMemorySyncBackend::new();
    backend.compare_and_swap("k", None, b"base").await.unwrap();

    // 两个设备都先读。
    let a_read = backend.read("k").await.unwrap().unwrap();
    let b_read = backend.read("k").await.unwrap().unwrap();

    // A 先写。
    backend
        .compare_and_swap("k", Some(a_read.revision), b"from-a")
        .await
        .unwrap();

    // B 照着自己读到的旧修订号写（这就是 naive caller 的做法）。
    let b_result = backend
        .compare_and_swap("k", Some(b_read.revision), b"from-b")
        .await;

    assert!(
        b_result.is_err(),
        "naive caller 也丢不了更新：写入是条件性的"
    );
    assert_eq!(
        backend.read("k").await.unwrap().unwrap().payload,
        b"from-a",
        "远端内容没有被覆盖"
    );
}

// ===========================================================================
// SyncBackend：错误返回
// ===========================================================================

#[tokio::test]
async fn revision_numbers_increase_monotonically() {
    let backend = InMemorySyncBackend::new();

    let r1 = backend.compare_and_swap("k", None, b"v1").await.unwrap();
    let r2 = backend
        .compare_and_swap("k", Some(r1), b"v2")
        .await
        .unwrap();
    let r3 = backend
        .compare_and_swap("k", Some(r2), b"v3")
        .await
        .unwrap();

    assert!(r1 < r2 && r2 < r3);
    assert_eq!(r3.next(), Revision(r3.0 + 1));
}

#[tokio::test]
async fn separate_keys_do_not_interfere() {
    let backend = InMemorySyncBackend::new();

    let a = backend.compare_and_swap("a", None, b"x").await.unwrap();
    let b = backend.compare_and_swap("b", None, b"y").await.unwrap();

    assert_eq!(backend.len(), 2);
    assert_ne!(a, b, "不同条目的修订号不应相同");
    assert_eq!(backend.read("a").await.unwrap().unwrap().payload, b"x");
    assert_eq!(backend.read("b").await.unwrap().unwrap().payload, b"y");
}

#[tokio::test]
async fn empty_payload_is_storable() {
    let backend = InMemorySyncBackend::new();

    backend.compare_and_swap("k", None, b"").await.unwrap();

    let entry = backend.read("k").await.unwrap().unwrap();
    assert!(entry.payload.is_empty());
}

#[tokio::test]
async fn remote_entry_is_a_plain_value() {
    // DTO 应当是普通值，便于跨层传递与断言。
    let entry = RemoteEntry {
        payload: vec![1, 2, 3],
        revision: Revision(7),
    };

    assert_eq!(entry.revision, Revision(7));
    assert_eq!(entry.payload.len(), 3);
}
