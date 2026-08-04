-- Diarch schema v1
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY NOT NULL,
    username TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    is_admin INTEGER NOT NULL DEFAULT 0,
    show_audio_gaps INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
    token TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS taxonomy (
    code INTEGER PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    parent_code INTEGER REFERENCES taxonomy(code)
);

CREATE TABLE IF NOT EXISTS works (
    id TEXT PRIMARY KEY NOT NULL,
    title TEXT NOT NULL,
    authors TEXT NOT NULL DEFAULT '',
    isbn TEXT,
    description TEXT,
    status TEXT NOT NULL DEFAULT 'unread',
    primary_code INTEGER REFERENCES taxonomy(code),
    year_list INTEGER,
    rating REAL,
    review TEXT,
    reading_direction TEXT NOT NULL DEFAULT 'ltr',
    is_manga INTEGER NOT NULL DEFAULT 0,
    needs_review INTEGER NOT NULL DEFAULT 0,
    needs_cover INTEGER NOT NULL DEFAULT 1,
    needs_tts INTEGER NOT NULL DEFAULT 0,
    needs_audio INTEGER NOT NULL DEFAULT 0,
    needs_transcription INTEGER NOT NULL DEFAULT 0,
    sg_review_dirty INTEGER NOT NULL DEFAULT 0,
    sg_needs_add INTEGER NOT NULL DEFAULT 0,
    sg_audio_only_remote INTEGER NOT NULL DEFAULT 0,
    sg_matched INTEGER NOT NULL DEFAULT 0,
    sg_book_id TEXT,
    created_by TEXT REFERENCES users(id),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS work_codes (
    work_id TEXT NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    code INTEGER NOT NULL REFERENCES taxonomy(code),
    PRIMARY KEY (work_id, code)
);

CREATE TABLE IF NOT EXISTS work_grants (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    work_id TEXT NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    PRIMARY KEY (user_id, work_id)
);

CREATE TABLE IF NOT EXISTS work_assets (
    id TEXT PRIMARY KEY NOT NULL,
    work_id TEXT NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    mime TEXT,
    bytes INTEGER,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS reading_progress (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    work_id TEXT NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    mode TEXT NOT NULL DEFAULT 'epub',
    position TEXT NOT NULL DEFAULT '',
    percent REAL NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (user_id, work_id, mode)
);

CREATE TABLE IF NOT EXISTS jobs (
    id TEXT PRIMARY KEY NOT NULL,
    kind TEXT NOT NULL,
    work_id TEXT REFERENCES works(id) ON DELETE SET NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    detail TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS integration_health (
    name TEXT PRIMARY KEY NOT NULL,
    status TEXT NOT NULL DEFAULT 'unknown',
    last_error TEXT,
    last_ok_at TEXT,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS user_settings (
    user_id TEXT PRIMARY KEY NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    reader_infinite_scroll INTEGER NOT NULL DEFAULT 0,
    json_extra TEXT NOT NULL DEFAULT '{}',
    reader_typography TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX IF NOT EXISTS idx_works_status ON works(status);
CREATE INDEX IF NOT EXISTS idx_works_primary ON works(primary_code);
CREATE INDEX IF NOT EXISTS idx_works_updated ON works(updated_at);
CREATE INDEX IF NOT EXISTS idx_works_created_by ON works(created_by);
CREATE INDEX IF NOT EXISTS idx_works_year_list ON works(year_list);
CREATE INDEX IF NOT EXISTS idx_works_needs_review ON works(needs_review);
CREATE INDEX IF NOT EXISTS idx_works_needs_cover ON works(needs_cover);
CREATE INDEX IF NOT EXISTS idx_works_needs_tts ON works(needs_tts);
CREATE INDEX IF NOT EXISTS idx_works_needs_audio ON works(needs_audio);
CREATE INDEX IF NOT EXISTS idx_works_needs_transcription ON works(needs_transcription);
CREATE INDEX IF NOT EXISTS idx_works_sg_review_dirty ON works(sg_review_dirty);
CREATE INDEX IF NOT EXISTS idx_works_sg_needs_add ON works(sg_needs_add);
CREATE INDEX IF NOT EXISTS idx_works_sg_audio_only_remote ON works(sg_audio_only_remote);
CREATE INDEX IF NOT EXISTS idx_work_assets_work ON work_assets(work_id);
CREATE INDEX IF NOT EXISTS idx_work_assets_work_kind ON work_assets(work_id, kind);
CREATE INDEX IF NOT EXISTS idx_jobs_work_kind ON jobs(work_id, kind);
CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions(user_id);
CREATE INDEX IF NOT EXISTS idx_jobs_status ON jobs(status);
CREATE INDEX IF NOT EXISTS idx_work_grants_work ON work_grants(work_id);
