-- Environment state shares the machine ledger, writer, archive and barriers.
-- Snapshot and proposal bytes live in event_body_blobs, never inline here.
CREATE TABLE IF NOT EXISTS environment_schema (
    id INTEGER PRIMARY KEY CHECK (id=1),
    version INTEGER NOT NULL CHECK (version>0)
);
INSERT OR IGNORE INTO environment_schema(id,version) VALUES(1,1);

CREATE TABLE IF NOT EXISTS environment_revisions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    environment_id TEXT NOT NULL,
    revision_id TEXT NOT NULL UNIQUE,
    previous_revision_id TEXT,
    fork_parent_environment_id TEXT,
    fork_parent_revision_id TEXT,
    timestamp_unix_ms INTEGER NOT NULL,
    actor TEXT NOT NULL,
    reason TEXT NOT NULL,
    request_id TEXT,
    state TEXT NOT NULL CHECK (state IN ('committing','committed','recovered_unapplied')),
    payload_event_id TEXT NOT NULL UNIQUE,
    replayable INTEGER NOT NULL DEFAULT 1 CHECK (replayable IN (0,1)),
    CHECK ((fork_parent_environment_id IS NULL) = (fork_parent_revision_id IS NULL)),
    UNIQUE(environment_id,revision_id),
    FOREIGN KEY(environment_id,previous_revision_id)
        REFERENCES environment_revisions(environment_id,revision_id),
    FOREIGN KEY(fork_parent_environment_id,fork_parent_revision_id)
        REFERENCES environment_revisions(environment_id,revision_id),
    FOREIGN KEY(request_id) REFERENCES environment_requests(request_id)
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_environment_revisions_inflight
    ON environment_revisions(environment_id) WHERE state='committing';
CREATE INDEX IF NOT EXISTS idx_environment_revisions_parent
    ON environment_revisions(previous_revision_id);
CREATE INDEX IF NOT EXISTS idx_environment_revisions_fork
    ON environment_revisions(fork_parent_revision_id);
CREATE INDEX IF NOT EXISTS idx_environment_revisions_time
    ON environment_revisions(timestamp_unix_ms,id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_environment_revisions_request
    ON environment_revisions(request_id) WHERE request_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS environment_requests (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL CHECK (kind IN ('change','image_update','promotion','apply')),
    environment_id TEXT NOT NULL,
    base_revision_id TEXT,
    source_environment_id TEXT,
    source_revision_id TEXT,
    requester_session_id TEXT,
    requester_generation TEXT,
    timestamp_unix_ms INTEGER NOT NULL,
    actor TEXT NOT NULL,
    reason TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('pending','approved','denied','cancelled')),
    payload_event_id TEXT NOT NULL UNIQUE,
    replayable INTEGER NOT NULL DEFAULT 1 CHECK (replayable IN (0,1)),
    result_environment_id TEXT,
    result_revision_id TEXT,
    CHECK ((requester_session_id IS NULL) = (requester_generation IS NULL)),
    CHECK ((result_environment_id IS NULL) = (result_revision_id IS NULL)),
    CHECK ((source_environment_id IS NULL) = (source_revision_id IS NULL)),
    CHECK ((kind IN ('promotion','apply')) = (source_revision_id IS NOT NULL)),
    FOREIGN KEY(environment_id,base_revision_id)
        REFERENCES environment_revisions(environment_id,revision_id),
    FOREIGN KEY(result_environment_id,result_revision_id)
        REFERENCES environment_revisions(environment_id,revision_id),
    FOREIGN KEY(source_environment_id,source_revision_id)
        REFERENCES environment_revisions(environment_id,revision_id)
);
CREATE INDEX IF NOT EXISTS idx_environment_requests_environment
    ON environment_requests(environment_id,id);
-- Historical requests are unbounded; admission counts only the bounded pending set.
CREATE INDEX IF NOT EXISTS idx_environment_requests_pending
    ON environment_requests(requester_session_id,id) WHERE state='pending';
CREATE INDEX IF NOT EXISTS idx_environment_requests_requester
    ON environment_requests(requester_session_id,requester_generation,id);
CREATE INDEX IF NOT EXISTS idx_environment_requests_source
    ON environment_requests(source_environment_id,source_revision_id);

CREATE TABLE IF NOT EXISTS environment_request_decisions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id TEXT NOT NULL UNIQUE REFERENCES environment_requests(request_id),
    timestamp_unix_ms INTEGER NOT NULL,
    actor TEXT NOT NULL,
    decision TEXT NOT NULL CHECK (decision IN ('approve','deny','cancel')),
    reason TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS environment_applications (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id TEXT NOT NULL REFERENCES environment_requests(request_id),
    session_id TEXT NOT NULL,
    generation TEXT NOT NULL,
    timestamp_unix_ms INTEGER NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('pending','applying','applied','failed')),
    boot_environment_id TEXT NOT NULL,
    boot_revision_id TEXT NOT NULL,
    live_policy_environment_id TEXT NOT NULL,
    live_policy_revision_id TEXT NOT NULL,
    desired_environment_id TEXT NOT NULL,
    desired_revision_id TEXT NOT NULL,
    corporate_policy_id TEXT NOT NULL,
    pending_restart INTEGER NOT NULL CHECK (pending_restart IN (0,1)),
    failure_reason TEXT,
    UNIQUE(request_id,session_id,generation),
    FOREIGN KEY(boot_environment_id,boot_revision_id)
        REFERENCES environment_revisions(environment_id,revision_id),
    FOREIGN KEY(live_policy_environment_id,live_policy_revision_id)
        REFERENCES environment_revisions(environment_id,revision_id),
    FOREIGN KEY(desired_environment_id,desired_revision_id)
        REFERENCES environment_revisions(environment_id,revision_id)
);
CREATE INDEX IF NOT EXISTS idx_environment_applications_session
    ON environment_applications(session_id,generation,id);

CREATE TABLE IF NOT EXISTS environment_revision_references (
    owner_kind TEXT NOT NULL CHECK (owner_kind IN (
        'current_environment','sandbox_boot','sandbox_live_policy','sandbox_desired',
        'pending_request','fork_lineage'
    )),
    owner_id TEXT NOT NULL,
    owner_generation TEXT NOT NULL,
    environment_id TEXT NOT NULL,
    revision_id TEXT NOT NULL,
    PRIMARY KEY(owner_kind,owner_id,owner_generation),
    FOREIGN KEY(environment_id,revision_id)
        REFERENCES environment_revisions(environment_id,revision_id)
);
CREATE INDEX IF NOT EXISTS idx_environment_revision_references_revision
    ON environment_revision_references(environment_id,revision_id);

CREATE TABLE IF NOT EXISTS environment_variable_purges (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    purge_id TEXT NOT NULL UNIQUE,
    environment_id TEXT NOT NULL,
    variable_name TEXT NOT NULL,
    timestamp_unix_ms INTEGER NOT NULL,
    actor TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('prepared','rewriting','committed','reclaim_pending','failed')),
    old_generation_id BLOB CHECK (old_generation_id IS NULL OR length(old_generation_id)=16),
    new_generation_id BLOB CHECK (new_generation_id IS NULL OR length(new_generation_id)=16),
    failure_reason TEXT
);
CREATE TABLE IF NOT EXISTS environment_revision_purges (
    purge_id TEXT NOT NULL REFERENCES environment_variable_purges(purge_id),
    revision_id TEXT NOT NULL REFERENCES environment_revisions(revision_id),
    PRIMARY KEY(purge_id,revision_id)
);
CREATE INDEX IF NOT EXISTS idx_environment_revision_purges_revision
    ON environment_revision_purges(revision_id,purge_id);
