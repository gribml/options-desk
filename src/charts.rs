//! Thin interop over uPlot (loaded from CDN in `index.html`) for the 2D line
//! charts on the Combos tab. Charts are keyed by their container element id;
//! re-plotting the same id destroys and recreates the instance, so Leptos
//! effects can just call `line_plot` whenever their inputs change.

use serde::Serialize;
use wasm_bindgen::prelude::*;

/// One line on a chart. `y` is aligned index-for-index with the shared `x`.
#[derive(Serialize)]
pub struct Series {
    pub label: String,
    pub color: String,
    pub y: Vec<f64>,
}

/// A 2D line chart. `x` is shared across all series. `*_fmt` control axis tick
/// and hover formatting: `"usd"`, `"pct"`, `"num2"`, or `"time"` (x only; sets a
/// date scale).
#[derive(Serialize)]
pub struct LinePlot {
    pub title: String,
    pub x_label: String,
    pub x_fmt: String,
    pub y_fmt: String,
    pub x: Vec<f64>,
    pub series: Vec<Series>,
}

#[wasm_bindgen(inline_js = r#"
const __otPlots = {};

// Dollar amount for an axis tick: full cents up to $10K, then $12.5K / $1.25M
// style — the same abbreviation the rest of the app uses on narrow screens.
// Up to two decimals with trailing zeros dropped, so uPlot's 1/2/2.5/5 tick
// steps never render two ticks with the same label.
function __otUsdShort(v) {
  const sign = v < 0 ? '-$' : '$';
  const a = Math.abs(v);
  const trim = x => x.toFixed(2).replace(/\.?0+$/, '');
  if (a >= 1e9) return sign + trim(a / 1e9) + 'B';
  if (a >= 1e6) return sign + trim(a / 1e6) + 'M';
  if (a >= 1e4) return sign + trim(a / 1e3) + 'K';
  return sign + a.toFixed(2);
}

function __otUsdFull(v) {
  return (v < 0 ? '-$' : '$') + Math.abs(v).toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 });
}

// `full` = tooltip (exact value); otherwise axis ticks (compact).
function __otFmt(kind, full) {
  switch (kind) {
    case 'usd':  return (u, vals) => vals.map(full ? __otUsdFull : __otUsdShort);
    case 'pct':  return (u, vals) => vals.map(v => (v * 100).toFixed(1) + '%');
    case 'num0': return (u, vals) => vals.map(v => v.toFixed(0));
    case 'num2': return (u, vals) => vals.map(v => v.toFixed(2));
    default:     return undefined; // uPlot default (date labels on a time scale)
  }
}

// Axis width from its longest tick label, so a six-figure value isn't cut
// off at a fixed size. uPlot re-runs this as ticks change; the cycle guard
// stops it re-measuring forever when a new width changes the tick set.
function __otAxisSize(u, values, axisIdx, cycleNum) {
  const axis = u.axes[axisIdx];
  if (cycleNum > 1) return axis._size;
  let size = axis.ticks.size + axis.gap;
  const longest = (values || []).reduce((a, v) => (v.length > a.length ? v : a), '');
  if (longest !== '') {
    u.ctx.font = axis.font[0];
    size += u.ctx.measureText(longest).width / devicePixelRatio;
  }
  return Math.ceil(size + 6);
}

export function otDestroyPlot(divId) {
  const inst = __otPlots[divId];
  if (inst) { inst.destroy(); delete __otPlots[divId]; }
}

// Floating tooltip that snaps to the (single) series and shows just its value.
function __otTooltip(fmtY) {
  let el;
  return {
    hooks: {
      init: u => {
        el = document.createElement('div');
        el.style.cssText = 'position:absolute;pointer-events:none;display:none;z-index:10;'
          + 'background:#0f1117;border:1px solid #2a2d3a;border-radius:4px;'
          + 'color:#cbd5e1;font-size:11px;padding:2px 6px;white-space:nowrap;'
          + 'transform:translate(-50%,-140%);';
        u.over.appendChild(el);
      },
      setCursor: u => {
        const idx = u.cursor.idx;
        const y = idx == null ? null : u.data[1][idx];
        if (y == null) { el.style.display = 'none'; return; }
        el.style.display = '';
        el.style.left = u.valToPos(u.data[0][idx], 'x') + 'px';
        el.style.top = u.valToPos(y, 'y') + 'px';
        el.textContent = fmtY ? fmtY(u, [y])[0] : String(y);
      },
    },
  };
}

export function otLinePlot(divId, payloadJson) {
  const el = document.getElementById(divId);
  if (!el || !window.uPlot) return;
  const p = JSON.parse(payloadJson);
  otDestroyPlot(divId);

  const isTime = p.x_fmt === 'time';
  const data = [p.x, ...p.series.map(s => s.y)];
  const grid = { stroke: '#2a2d3a', width: 1 };

  const fmtX = isTime ? null : __otFmt(p.x_fmt);
  const fmtY = __otFmt(p.y_fmt);
  const fmtYFull = __otFmt(p.y_fmt, true);

  const opts = {
    width: el.clientWidth || 320,
    height: 240,
    title: p.title,
    cursor: { drag: { x: true, y: false }, points: { size: 6 } },
    scales: { x: { time: isTime } },
    axes: [
      { label: p.x_label, stroke: '#9ca3af', grid, ticks: grid, values: fmtX },
      { stroke: '#9ca3af', grid, ticks: grid, size: __otAxisSize, values: fmtY },
    ],
    series: [
      {},
      ...p.series.map(s => ({ label: s.label, stroke: s.color, width: 2, points: { show: false } })),
    ],
    legend: { show: false },
    plugins: [__otTooltip(fmtYFull)],
  };
  __otPlots[divId] = new window.uPlot(opts, data, el);
}
"#)]
extern "C" {
    #[wasm_bindgen(js_name = otLinePlot)]
    fn ot_line_plot_js(div_id: &str, payload_json: &str);
    #[wasm_bindgen(js_name = otDestroyPlot)]
    fn ot_destroy_plot_js(div_id: &str);
}

/// Render (or re-render) a line chart into the element with id `div_id`.
pub fn line_plot(div_id: &str, plot: &LinePlot) {
    if let Ok(json) = serde_json::to_string(plot) {
        ot_line_plot_js(div_id, &json);
    }
}

/// Destroy the chart instance in `div_id`, if any (call on unmount).
pub fn destroy_plot(div_id: &str) {
    ot_destroy_plot_js(div_id);
}
