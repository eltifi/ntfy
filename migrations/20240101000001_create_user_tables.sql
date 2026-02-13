-- Migration to create user tables, ported from Go implementation
-- Corresponds to `createTablesQueries` in go/user/manager.go

CREATE TABLE IF NOT EXISTS tier (
    id TEXT PRIMARY KEY,
    code TEXT NOT NULL,
    name TEXT NOT NULL,
    messages_limit INT NOT NULL,
    messages_expiry_duration INT NOT NULL,
    emails_limit INT NOT NULL,
    calls_limit INT NOT NULL,
    reservations_limit INT NOT NULL,
    attachment_file_size_limit INT NOT NULL,
    attachment_total_size_limit INT NOT NULL,
    attachment_expiry_duration INT NOT NULL,
    attachment_bandwidth_limit INT NOT NULL,
    stripe_monthly_price_id TEXT,
    stripe_yearly_price_id TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_tier_code ON tier (code);
CREATE UNIQUE INDEX IF NOT EXISTS idx_tier_stripe_monthly_price_id ON tier (stripe_monthly_price_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_tier_stripe_yearly_price_id ON tier (stripe_yearly_price_id);

CREATE TABLE IF NOT EXISTS user (
    id TEXT PRIMARY KEY,
    tier_id TEXT,
    user TEXT NOT NULL,
    pass TEXT NOT NULL,
    role TEXT CHECK (role IN ('anonymous', 'admin', 'user')) NOT NULL,
    prefs JSON NOT NULL DEFAULT '{}',
    sync_topic TEXT NOT NULL,
    provisioned INT NOT NULL,
    stats_messages INT NOT NULL DEFAULT (0),
    stats_emails INT NOT NULL DEFAULT (0),
    stats_calls INT NOT NULL DEFAULT (0),
    stripe_customer_id TEXT,
    stripe_subscription_id TEXT,
    stripe_subscription_status TEXT,
    stripe_subscription_interval TEXT,
    stripe_subscription_paid_until INT,
    stripe_subscription_cancel_at INT,
    created INT NOT NULL,
    deleted INT,
    FOREIGN KEY (tier_id) REFERENCES tier (id)
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_user ON user (user);
CREATE UNIQUE INDEX IF NOT EXISTS idx_user_stripe_customer_id ON user (stripe_customer_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_user_stripe_subscription_id ON user (stripe_subscription_id);

CREATE TABLE IF NOT EXISTS user_access (
    user_id TEXT NOT NULL,
    topic TEXT NOT NULL,
    read INT NOT NULL,
    write INT NOT NULL,
    owner_user_id INT,
    provisioned INT NOT NULL,
    PRIMARY KEY (user_id, topic),
    FOREIGN KEY (user_id) REFERENCES user (id) ON DELETE CASCADE,
    FOREIGN KEY (owner_user_id) REFERENCES user (id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS user_token (
    user_id TEXT NOT NULL,
    token TEXT NOT NULL,
    label TEXT NOT NULL,
    last_access INT NOT NULL,
    last_origin TEXT NOT NULL,
    expires INT NOT NULL,
    provisioned INT NOT NULL,
    PRIMARY KEY (user_id, token),
    FOREIGN KEY (user_id) REFERENCES user (id) ON DELETE CASCADE
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_user_token ON user_token (token);

CREATE TABLE IF NOT EXISTS user_phone (
    user_id TEXT NOT NULL,
    phone_number TEXT NOT NULL,
    PRIMARY KEY (user_id, phone_number),
    FOREIGN KEY (user_id) REFERENCES user (id) ON DELETE CASCADE
);

-- Note: 'schemaVersion' table might already exist from store.rs, but that's for messages.
-- Go implementation seems to use the same DB for both in some configs, or separates them.
-- If we use the same DB, we need to be careful about schema versioning collision if they use the same table name.
-- In Go `message_cache.go`, it uses `schemaVersion`. In `user/manager.go`, it ALSO uses `schemaVersion`!
-- If they are in the same DB, this is a conflict. 
-- For this Rust port, we will assume they share the DB if `cache_file` == `auth_file`, so we must check if `schemaVersion` works.
-- Actually, since we are doing a "Clean" port, we might just use sqlx migrations which handle versioning via `_sqlx_migrations` table.
-- But for compatibility with existing Go DBs, we should probably respect their schema.
-- However, sqlx migrations are way easier.
-- Let's stick to sqlx migrations for now, as we are creating a new server.
-- If the user wants to migrate data, they might need a separate tool or manual migration.
-- We will use this file as a standard sqlx migration.
