-- Per-account shelf placement. Safe to re-run (IF NOT EXISTS / OR IGNORE).

CREATE TABLE IF NOT EXISTS user_work_status (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    work_id TEXT NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'unread',
    updated_at TEXT NOT NULL,
    PRIMARY KEY (user_id, work_id)
);

CREATE INDEX IF NOT EXISTS idx_user_work_status_user_status ON user_work_status(user_id, status);
CREATE INDEX IF NOT EXISTS idx_user_work_status_work ON user_work_status(work_id);

-- Seed creators with the previous global works.status.
INSERT OR IGNORE INTO user_work_status (user_id, work_id, status, updated_at)
SELECT created_by, id, status, updated_at FROM works WHERE created_by IS NOT NULL;

-- Seed grantees so shared libraries keep their prior shelf placement.
INSERT OR IGNORE INTO user_work_status (user_id, work_id, status, updated_at)
SELECT g.user_id, w.id, w.status, w.updated_at
FROM work_grants g
JOIN works w ON w.id = g.work_id;

-- Seed admins (they see all works) from the prior global status.
INSERT OR IGNORE INTO user_work_status (user_id, work_id, status, updated_at)
SELECT u.id, w.id, w.status, w.updated_at
FROM users u
CROSS JOIN works w
WHERE u.is_admin = 1;
