use super::*;
use crate::testdb::{count, setup};

#[tokio::test]
async fn an_entry_is_put_found_and_forgotten() {
    let Some((_db, store)) = setup().await else { return };
    let cache = store.cache();
    assert_eq!(cache.get("k1").await.expect("get"), None);
    cache.put("k1", "/cache/k1.bin", 100, Tier::Preview).await.expect("put");
    let e = cache.get("k1").await.expect("get").expect("there");
    assert_eq!((e.file.as_str(), e.size_bytes, e.tier), ("/cache/k1.bin", 100, Tier::Preview));
    cache.put("k1", "/cache/k1b.bin", 150, Tier::Final).await.expect("replace");
    assert_eq!(cache.get("k1").await.expect("get").map(|e| (e.size_bytes, e.tier)), Some((150, Tier::Final)));
    assert_eq!(cache.total_bytes(None).await.expect("total"), 150);
    assert_eq!(cache.total_bytes(Some(Tier::Preview)).await.expect("total"), 0);
    assert!(cache.remove("k1").await.expect("remove"));
    assert!(!cache.remove("k1").await.expect("remove twice"));
}

#[tokio::test]
async fn eviction_takes_the_least_recently_used_until_the_budget_fits() {
    let Some((_db, store)) = setup().await else { return };
    let cache = store.cache();
    for (i, key) in ["a", "b", "c", "d"].iter().enumerate() {
        cache.put(key, &format!("/cache/{key}"), 100, Tier::Preview).await.expect("put");
        // distinct, ordered times
        let client = store.client().await.expect("client");
        client
            .execute("UPDATE cache.entry SET last_used_at = now() - make_interval(secs => $2) WHERE key = $1", &[key, &(100.0 - i as f64)])
            .await
            .expect("age");
    }
    // a is the oldest; touching it makes b the oldest
    cache.get("a").await.expect("touch");
    let gone = cache.evict_to(250, None).await.expect("evict");
    assert_eq!(gone.iter().map(|e| e.key.as_str()).collect::<Vec<_>>(), ["b", "c"]);
    assert_eq!(cache.total_bytes(None).await.expect("total"), 200);
    assert_eq!(cache.get("b").await.expect("get"), None);
    assert!(cache.evict_to(1_000, None).await.expect("fits").is_empty());
    assert_eq!(cache.evict_to(0, None).await.expect("all").len(), 2);
    assert_eq!(count(&store, "SELECT count(*) FROM cache.entry").await, 0);
}

#[tokio::test]
async fn eviction_by_tier_leaves_the_other_tier_alone() {
    let Some((_db, store)) = setup().await else { return };
    let cache = store.cache();
    cache.put("p1", "/cache/p1", 100, Tier::Preview).await.expect("put");
    cache.put("p2", "/cache/p2", 100, Tier::Preview).await.expect("put");
    cache.put("f1", "/cache/f1", 500, Tier::Final).await.expect("put");
    let gone = cache.evict_to(0, Some(Tier::Preview)).await.expect("evict");
    assert_eq!(gone.len(), 2);
    assert_eq!(cache.total_bytes(None).await.expect("total"), 500);
}

#[tokio::test]
async fn hostile_input_is_refused() {
    let Some((_db, store)) = setup().await else { return };
    let cache = store.cache();
    assert!(cache.put("", "/f", 1, Tier::Final).await.is_err());
    assert!(cache.put(&"k".repeat(257), "/f", 1, Tier::Final).await.is_err());
    assert!(cache.put("k", "", 1, Tier::Final).await.is_err());
    assert!(cache.put("k", "/f", u64::MAX, Tier::Final).await.is_err());
    assert!(cache.get("nul\0").await.is_err());
    assert_eq!(count(&store, "SELECT count(*) FROM cache.entry").await, 0);
}

#[tokio::test]
async fn the_cache_schema_migrates_back_and_forth() {
    let Some((_db, store)) = setup().await else { return };
    assert_eq!(store.rollback("cache").await.expect("down"), Some(1));
    assert_eq!(count(&store, "SELECT count(*) FROM pg_namespace WHERE nspname = 'cache'").await, 0);
    assert_eq!(store.migrate().await.expect("up"), vec![("cache".to_owned(), 1)]);
}
