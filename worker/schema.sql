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

-- SABR surface written by the Dagster calibration pipeline.
CREATE TABLE IF NOT EXISTS vol_surface (
    underlying    TEXT NOT NULL,
    snapshot_date TEXT NOT NULL,  -- YYYY-MM-DD
    expiry        TEXT NOT NULL,  -- YYYY-MM-DD
    alpha         REAL NOT NULL,
    beta          REAL NOT NULL,  -- fixed at 1.0 (log-normal SABR)
    rho           REAL NOT NULL,
    nu            REAL NOT NULL,
    atm_vol       REAL NOT NULL,  -- pre-computed ATM vol; used for variance-curve interpolation
    forward       REAL NOT NULL,  -- forward price at calibration time
    PRIMARY KEY (underlying, snapshot_date, expiry)
);

CREATE INDEX IF NOT EXISTS idx_surface_underlying_snapshot
    ON vol_surface (underlying, snapshot_date DESC);

-- Implied risk-free rates per expiry, backed out from ATM options by the data pipeline.
-- One row per (underlying, snapshot_date, expiry); queried by /term-rates.
CREATE TABLE IF NOT EXISTS implied_rates (
    underlying    TEXT NOT NULL,
    snapshot_date TEXT NOT NULL,  -- YYYY-MM-DD
    expiry        TEXT NOT NULL,  -- YYYY-MM-DD
    rate          REAL NOT NULL,  -- annualised risk-free rate (e.g. 0.0525 = 5.25%)
    num_contracts INTEGER,        -- number of ATM contracts used in the average
    PRIMARY KEY (underlying, snapshot_date, expiry)
);

CREATE INDEX IF NOT EXISTS idx_implied_rates_underlying_snapshot
    ON implied_rates (underlying, snapshot_date DESC);

-- Real-time cache populated by the /latest-bar worker route.
-- One row per ticker; upserted on every fresh Alpaca fetch.
CREATE TABLE IF NOT EXISTS latest_bars_cache (
    symbol      TEXT NOT NULL,
    bar_time    TEXT NOT NULL,  -- ISO 8601 timestamp of the bar itself (from Alpaca)
    fetched_at  TEXT NOT NULL,   -- ISO 8601 UTC timestamp of when we fetched from Alpaca
    open        REAL,
    high        REAL,
    low         REAL,
    close       REAL,
    volume      REAL,
    trade_count INTEGER,
    vwap        REAL,
    PRIMARY KEY (symbol)
);

-- On-demand option-chain cache populated by the /option-chain-live worker route.
-- Mirrors latest_bars_cache: one row per contract, upserted on every fresh Alpaca
-- fetch, with a 15-minute freshness window. Distinct from the pipeline-populated
-- `option_chain` table (which carries calibrated greeks for known symbols).
CREATE TABLE IF NOT EXISTS option_chain_cache (
    symbol             TEXT NOT NULL,  -- OCC contract symbol (e.g. AAPL250117C00150000)
    underlying         TEXT NOT NULL,  -- ticker
    fetched_at         TEXT NOT NULL,  -- ISO 8601 UTC timestamp of the Alpaca fetch
    expiration         TEXT,           -- YYYY-MM-DD
    option_type        TEXT,           -- 'call' | 'put'
    strike             REAL,
    bid                REAL,
    ask                REAL,
    last_price         REAL,
    implied_volatility REAL,
    delta              REAL,
    gamma              REAL,
    theta              REAL,
    vega               REAL,
    rho                REAL,
    PRIMARY KEY (symbol)
);

CREATE INDEX IF NOT EXISTS idx_chain_cache_underlying_fetched
    ON option_chain_cache (underlying, fetched_at DESC);
