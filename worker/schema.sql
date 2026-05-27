CREATE TABLE IF NOT EXISTS bars_1min (
    symbol  TEXT    NOT NULL,
    ts      TEXT    NOT NULL,  -- ISO 8601 timestamp
    open    REAL    NOT NULL,
    high    REAL    NOT NULL,
    low     REAL    NOT NULL,
    close   REAL    NOT NULL,
    volume  INTEGER,
    PRIMARY KEY (symbol, ts)
);

CREATE TABLE IF NOT EXISTS option_chain (
    snapshot_time  TEXT    NOT NULL,  -- ISO 8601 timestamp
    symbol         TEXT    NOT NULL,  -- OCC symbol e.g. AAPL250117C00150000
    underlying     TEXT    NOT NULL,
    expiry         TEXT    NOT NULL,  -- YYYY-MM-DD
    option_type    TEXT    NOT NULL,  -- 'call' | 'put'
    strike         REAL    NOT NULL,
    bid            REAL,
    ask            REAL,
    mid            REAL,
    implied_vol    REAL,
    delta          REAL,
    gamma          REAL,
    theta          REAL,
    vega           REAL,
    open_interest  INTEGER,
    volume         INTEGER,
    PRIMARY KEY (snapshot_time, symbol)
);

CREATE INDEX IF NOT EXISTS idx_bars_symbol_ts
    ON bars_1min (symbol, ts DESC);

CREATE INDEX IF NOT EXISTS idx_chain_underlying_snapshot
    ON option_chain (underlying, snapshot_time DESC);

CREATE INDEX IF NOT EXISTS idx_chain_underlying_expiry
    ON option_chain (underlying, expiry, option_type, strike);
