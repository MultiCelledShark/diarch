use chrono::Utc;
use diarch_core::{ReadingProgress, ReadingStatus, Work};
use diarch_db::Db;
use tempfile::tempdir;
use uuid::Uuid;

async fn fresh_db() -> (tempfile::TempDir, Db) {
    let dir = tempdir().unwrap();
    let db = Db::connect(&dir.path().join("t.db")).await.unwrap();
    let seed = include_str!("../../../taxonomy/seed.json");
    db.seed_taxonomy(seed).await.unwrap();
    (dir, db)
}

fn sample_work(created_by: Option<Uuid>, status: ReadingStatus) -> Work {
    let now = Utc::now();
    Work {
        id: Uuid::new_v4(),
        title: "Test Book".into(),
        authors: "Author".into(),
        isbn: Some("9780000000000".into()),
        description: None,
        status,
        primary_code: Some(8201),
        year_list: None,
        rating: None,
        review: None,
        reading_direction: "ltr".into(),
        is_manga: false,
        needs_review: false,
        needs_cover: true,
        needs_tts: false,
        needs_audio: false,
        needs_transcription: false,
        sg_review_dirty: false,
        sg_needs_add: false,
        sg_audio_only_remote: false,
        sg_matched: false,
        sg_book_id: None,
        created_by,
        created_at: now,
        updated_at: now,
    }
}

#[tokio::test]
async fn admin_bootstrap_and_password_verify() {
    let (_dir, db) = fresh_db().await;
    let admin = db
        .ensure_admin("admin", "secretpassword", None)
        .await
        .unwrap();
    assert!(admin.is_admin);
    // Existing admin password must not change without force.
    let again = db
        .ensure_admin("admin", "otherpasswordxx", None)
        .await
        .unwrap();
    assert_eq!(admin.id, again.id);
    let hash = db.get_password_hash("admin").await.unwrap().unwrap();
    assert!(Db::verify_password("secretpassword", &hash).unwrap());
    assert!(!Db::verify_password("otherpasswordxx", &hash).unwrap());
    // Opt-in force rotate.
    let _ = db
        .ensure_admin("admin", "ignored", Some("rotatedpass12"))
        .await
        .unwrap();
    let hash2 = db.get_password_hash("admin").await.unwrap().unwrap();
    assert!(Db::verify_password("rotatedpass12", &hash2).unwrap());
}

#[tokio::test]
async fn sessions_expire_lookup() {
    let (_dir, db) = fresh_db().await;
    let user = db.create_user("u1", "pw", false).await.unwrap();
    let token = db.create_session(user.id, 1).await.unwrap();
    let found = db.user_for_session(&token).await.unwrap().unwrap();
    assert_eq!(found.username, "u1");
    db.delete_session(&token).await.unwrap();
    assert!(db.user_for_session(&token).await.unwrap().is_none());
}

#[tokio::test]
async fn acl_hides_ungranted_works() {
    let (_dir, db) = fresh_db().await;
    let admin = db.create_user("admin", "pw", true).await.unwrap();
    let reader = db.create_user("reader", "pw", false).await.unwrap();

    let secret = sample_work(Some(admin.id), ReadingStatus::Unread);
    db.create_work(&secret, &[8201], None).await.unwrap();

    let shared = sample_work(Some(admin.id), ReadingStatus::Wishlist);
    let mut shared = shared;
    shared.title = "Shared".into();
    db.create_work(&shared, &[8201, 8940], Some(reader.id))
        .await
        .unwrap();

    let reader_works = db.list_works_for_user(&reader, None, None).await.unwrap();
    assert_eq!(reader_works.len(), 1);
    assert_eq!(reader_works[0].title, "Shared");
    assert!(!db.user_can_access(&reader, secret.id).await.unwrap());
    assert!(db.user_can_access(&reader, shared.id).await.unwrap());
    assert!(db.user_can_access(&admin, secret.id).await.unwrap());
}

#[tokio::test]
async fn multi_codes_and_attention_filters() {
    let (_dir, db) = fresh_db().await;
    let admin = db.create_user("admin", "pw", true).await.unwrap();
    let mut work = sample_work(Some(admin.id), ReadingStatus::Unread);
    work.needs_tts = true;
    work.sg_review_dirty = true;
    db.create_work(&work, &[8201, 8940], Some(admin.id))
        .await
        .unwrap();
    let codes = db.work_codes(work.id).await.unwrap();
    assert_eq!(codes, vec![8201, 8940]);

    db.set_work_codes(work.id, &[8403]).await.unwrap();
    assert_eq!(db.work_codes(work.id).await.unwrap(), vec![8403]);

    let tts = db
        .list_works_for_user(&admin, None, Some("needs_tts"))
        .await
        .unwrap();
    assert_eq!(tts.len(), 1);
    let dirty = db
        .list_works_for_user(&admin, None, Some("sg_review_dirty"))
        .await
        .unwrap();
    assert_eq!(dirty.len(), 1);
}

#[tokio::test]
async fn reading_progress_upsert() {
    let (_dir, db) = fresh_db().await;
    let user = db.create_user("u", "pw", false).await.unwrap();
    let work = sample_work(Some(user.id), ReadingStatus::Reading);
    db.create_work(&work, &[], Some(user.id)).await.unwrap();
    let p = ReadingProgress {
        user_id: user.id,
        work_id: work.id,
        mode: "epub".into(),
        position: "epubcfi(/6/2)".into(),
        percent: 12.5,
        updated_at: Utc::now(),
    };
    db.upsert_progress(&p).await.unwrap();
    let mut p2 = p.clone();
    p2.percent = 50.0;
    p2.position = "epubcfi(/6/4)".into();
    db.upsert_progress(&p2).await.unwrap();
    let got = db.get_progress(user.id, work.id, "epub").await.unwrap().unwrap();
    assert_eq!(got.percent, 50.0);
}

#[tokio::test]
async fn jobs_and_integration_health() {
    let (_dir, db) = fresh_db().await;
    let job = db.create_job("import", None).await.unwrap();
    assert_eq!(job.status, "pending");
    let next = db.next_pending_job().await.unwrap().unwrap();
    assert_eq!(next.id, job.id);
    db.update_job(job.id, "done", Some("ok")).await.unwrap();
    assert!(db.next_pending_job().await.unwrap().is_none());

    db.set_integration_health("pandoc", "ok", None, true)
        .await
        .unwrap();
    let health = db.list_integration_health().await.unwrap();
    assert!(health.iter().any(|h| h.name == "pandoc" && h.status == "ok"));
}

#[tokio::test]
async fn user_work_status_is_per_account() {
    let (_dir, db) = fresh_db().await;
    let admin = db.create_user("admin", "pw", true).await.unwrap();
    let reader = db.create_user("reader", "pw", false).await.unwrap();
    let work = sample_work(Some(admin.id), ReadingStatus::Reading);
    db.create_work(&work, &[], Some(reader.id)).await.unwrap();

    let admin_reading = db
        .list_works_for_user(&admin, Some("reading"), None)
        .await
        .unwrap();
    assert_eq!(admin_reading.len(), 1);

    let reader_reading = db
        .list_works_for_user(&reader, Some("reading"), None)
        .await
        .unwrap();
    assert!(reader_reading.is_empty());

    db.upsert_user_work_status(reader.id, work.id, "to_read")
        .await
        .unwrap();
    let reader_to_read = db
        .list_works_for_user(&reader, Some("to_read"), None)
        .await
        .unwrap();
    assert_eq!(reader_to_read.len(), 1);
    assert_eq!(reader_to_read[0].status, ReadingStatus::ToRead);

    // Admin still on Currently Reading.
    let admin_reading = db
        .list_works_for_user(&admin, Some("reading"), None)
        .await
        .unwrap();
    assert_eq!(admin_reading.len(), 1);
    assert_eq!(
        db.count_accessible_by_status(&admin, "reading", None)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        db.count_accessible_by_status(&reader, "reading", None)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn list_works_text_search_matches_title_author_isbn() {
    let (_dir, db) = fresh_db().await;
    let user = db.create_user("u", "pw", false).await.unwrap();

    let mut dune = sample_work(Some(user.id), ReadingStatus::Unread);
    dune.title = "Dune".into();
    dune.authors = "Frank Herbert".into();
    dune.isbn = Some("9780441172719".into());
    db.create_work(&dune, &[], None).await.unwrap();

    let mut other = sample_work(Some(user.id), ReadingStatus::Unread);
    other.title = "Neuromancer".into();
    other.authors = "William Gibson".into();
    other.isbn = Some("9780441569595".into());
    db.create_work(&other, &[], None).await.unwrap();

    let by_title = db
        .list_works_for_user_filtered(&user, None, None, None, Some("dune"))
        .await
        .unwrap();
    assert_eq!(by_title.len(), 1);
    assert_eq!(by_title[0].title, "Dune");

    let by_author = db
        .list_works_for_user_filtered(&user, None, None, None, Some("herbert"))
        .await
        .unwrap();
    assert_eq!(by_author.len(), 1);

    let by_isbn = db
        .list_works_for_user_filtered(&user, None, None, None, Some("441172719"))
        .await
        .unwrap();
    assert_eq!(by_isbn.len(), 1);

    let miss = db
        .list_works_for_user_filtered(&user, None, None, None, Some("asimov"))
        .await
        .unwrap();
    assert!(miss.is_empty());
}

#[tokio::test]
async fn created_by_grants_owner_access_without_explicit_grant() {
    let (_dir, db) = fresh_db().await;
    let user = db.create_user("owner", "pw", false).await.unwrap();
    let work = sample_work(Some(user.id), ReadingStatus::Wishlist);
    db.create_work(&work, &[9000], None).await.unwrap();
    assert!(db.user_can_access(&user, work.id).await.unwrap());
    let listed = db.list_works_for_user(&user, Some("wishlist"), None).await.unwrap();
    assert_eq!(listed.len(), 1);
}

#[tokio::test]
async fn delete_work_cascades_and_hides() {
    let (_dir, db) = fresh_db().await;
    let admin = db.create_user("admin", "pw", true).await.unwrap();
    let work = sample_work(Some(admin.id), ReadingStatus::Unread);
    db.create_work(&work, &[8201], Some(admin.id)).await.unwrap();
    db.create_job("import", Some(work.id)).await.unwrap();
    assert!(db.get_work(work.id).await.unwrap().is_some());
    assert!(db.delete_work(work.id).await.unwrap());
    assert!(db.get_work(work.id).await.unwrap().is_none());
    assert!(!db.delete_work(work.id).await.unwrap());
    assert!(diarch_db::Db::user_can_delete(&admin, &work));
}
