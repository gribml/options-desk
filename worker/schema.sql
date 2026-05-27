CREATE TABLE IF NOT EXISTS bars_1min (
    symbol     TEXT  NOT NULL,
    timestamp  TEXT  NOT NULL,  -- ISO 8601 timestamp
    open       REAL,
    high       REAL,
    low        REAL,
    close      REAL,
    volume     REAL,
    trade_count  INTEGER,
    vwap       REAL,
    PRIMARY KEY (symbol, timestamp)
);

CREATE TABLE IF NOT EXISTS option_chain (
    symbol             TEXT  NOT NULL,
    snapshot_date      TEXT  NOT NULL,  -- YYYY-MM-DD
    underlying         TEXT,
    expiration         TEXT,            -- YYYY-MM-DD
    option_type        TEXT,            -- 'call' | 'put'
    strike             REAL,
    bid                REAL,
    ask                REAL,
    bid_size           REAL,
    ask_size           REAL,
    last_price         REAL,
    last_size          REAL,
    implied_volatility REAL,
    delta              REAL,
    gamma              REAL,
    theta              REAL,
    vega               REAL,
    rho                REAL,
    PRIMARY KEY (symbol, snapshot_date)
);

CREATE INDEX IF NOT EXISTS idx_bars_symbol_ts
    ON bars_1min (symbol, timestamp DESC);

CREATE INDEX IF NOT EXISTS idx_chain_underlying_snapshot
    ON option_chain (underlying, snapshot_date DESC);

CREATE INDEX IF NOT EXISTS idx_chain_underlying_expiry
    ON option_chain (underlying, expiration, option_type, strike);
