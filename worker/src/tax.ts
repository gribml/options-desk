// ── Federal tax engine ──────────────────────────────────────────────────────────
//
// SCOPE: U.S. FEDERAL income tax on an individual return, to the level of detail
// needed to price the tax drag on a trade. Modelled:
//   • ordinary-income tax at the §1 progressive rates
//   • §1(h) preferential rates (0/15/20%) on net capital gain + qualified
//     dividends, stacked on top of ordinary taxable income
//   • §1211(b) capital-loss limitation and §1222 ST/LT netting of carryforwards
//   • §1411 Net Investment Income Tax (3.8%)
//
// OUT OF SCOPE (deliberately — none of these move the marginal tax on a stock or
// option trade, or they need data this app does not hold):
//   • AMT (§55), state and local tax
//   • the 0.9% Additional Medicare Tax (§3101(b)(2)) — wages/SE income only
//   • credits, phase-outs (QBI §199A, itemized-deduction limits), NOLs
//   • §1256 60/40 contracts, §1091 wash sales, §1092 straddles, §1259
//     constructive sales, §1233/§246(c) holding-period suspension. Callers pass
//     already-characterised st_gain / lt_gain, so these must be applied upstream.
//   • §469 passive-activity loss limits on rental losses
//   • the unused portion of a net capital loss carrying forward to a later year
//     (§1212(b)) — this is a single-year computation

export type Filing = 'single' | 'mfj' | 'mfs' | 'hoh';

export interface Bracket {
  upTo: number; // inclusive upper bound of this band; Infinity for the top band
  rate: number;
}

export interface YearConstants {
  stdDeduction: Record<Filing, number>;
  ordinary: Record<Filing, Bracket[]>;
  ltcg: Record<Filing, Bracket[]>;
  niitThreshold: Record<Filing, number>;
}

export const NIIT_RATE = 0.038;

// §1411(b) MAGI thresholds are fixed by statute (not inflation-adjusted).
export const NIIT_THRESHOLD: Record<Filing, number> = {
  single: 200_000,
  mfj: 250_000,
  mfs: 125_000,
  hoh: 200_000,
};

// ── FEDERAL TAX CONSTANTS BY YEAR (UPDATE YEARLY) ──
// 2025 = Rev. Proc. 2024-40, with the standard deduction as amended by OBBBA
// (P.L. 119-21 §70102); 2026 = Rev. Proc. 2025-32.
export const TAX_CONSTANTS: Record<number, YearConstants> = {
  2025: {
    stdDeduction: { single: 15_750, mfj: 31_500, mfs: 15_750, hoh: 23_625 },
    ordinary: {
      single: [
        { upTo: 11_925, rate: 0.10 }, { upTo: 48_475, rate: 0.12 },
        { upTo: 103_350, rate: 0.22 }, { upTo: 197_300, rate: 0.24 },
        { upTo: 250_525, rate: 0.32 }, { upTo: 626_350, rate: 0.35 },
        { upTo: Infinity, rate: 0.37 },
      ],
      mfj: [
        { upTo: 23_850, rate: 0.10 }, { upTo: 96_950, rate: 0.12 },
        { upTo: 206_700, rate: 0.22 }, { upTo: 394_600, rate: 0.24 },
        { upTo: 501_050, rate: 0.32 }, { upTo: 751_600, rate: 0.35 },
        { upTo: Infinity, rate: 0.37 },
      ],
      mfs: [
        { upTo: 11_925, rate: 0.10 }, { upTo: 48_475, rate: 0.12 },
        { upTo: 103_350, rate: 0.22 }, { upTo: 197_300, rate: 0.24 },
        { upTo: 250_525, rate: 0.32 }, { upTo: 375_800, rate: 0.35 },
        { upTo: Infinity, rate: 0.37 },
      ],
      hoh: [
        { upTo: 17_000, rate: 0.10 }, { upTo: 64_850, rate: 0.12 },
        { upTo: 103_350, rate: 0.22 }, { upTo: 197_300, rate: 0.24 },
        { upTo: 250_500, rate: 0.32 }, { upTo: 626_350, rate: 0.35 },
        { upTo: Infinity, rate: 0.37 },
      ],
    },
    ltcg: {
      single: [{ upTo: 48_350, rate: 0.0 }, { upTo: 533_400, rate: 0.15 }, { upTo: Infinity, rate: 0.20 }],
      mfj: [{ upTo: 96_700, rate: 0.0 }, { upTo: 600_050, rate: 0.15 }, { upTo: Infinity, rate: 0.20 }],
      mfs: [{ upTo: 48_350, rate: 0.0 }, { upTo: 300_000, rate: 0.15 }, { upTo: Infinity, rate: 0.20 }],
      hoh: [{ upTo: 64_750, rate: 0.0 }, { upTo: 566_700, rate: 0.15 }, { upTo: Infinity, rate: 0.20 }],
    },
    niitThreshold: NIIT_THRESHOLD,
  },
  2026: {
    stdDeduction: { single: 16_100, mfj: 32_200, mfs: 16_100, hoh: 24_150 },
    ordinary: {
      single: [
        { upTo: 12_400, rate: 0.10 }, { upTo: 50_400, rate: 0.12 },
        { upTo: 105_700, rate: 0.22 }, { upTo: 201_775, rate: 0.24 },
        { upTo: 256_225, rate: 0.32 }, { upTo: 640_600, rate: 0.35 },
        { upTo: Infinity, rate: 0.37 },
      ],
      mfj: [
        { upTo: 24_800, rate: 0.10 }, { upTo: 100_800, rate: 0.12 },
        { upTo: 211_400, rate: 0.22 }, { upTo: 403_550, rate: 0.24 },
        { upTo: 512_450, rate: 0.32 }, { upTo: 768_700, rate: 0.35 },
        { upTo: Infinity, rate: 0.37 },
      ],
      mfs: [
        { upTo: 12_400, rate: 0.10 }, { upTo: 50_400, rate: 0.12 },
        { upTo: 105_700, rate: 0.22 }, { upTo: 201_775, rate: 0.24 },
        { upTo: 256_225, rate: 0.32 }, { upTo: 384_350, rate: 0.35 },
        { upTo: Infinity, rate: 0.37 },
      ],
      hoh: [
        { upTo: 17_700, rate: 0.10 }, { upTo: 67_450, rate: 0.12 },
        { upTo: 105_700, rate: 0.22 }, { upTo: 201_775, rate: 0.24 },
        { upTo: 256_200, rate: 0.32 }, { upTo: 640_600, rate: 0.35 },
        { upTo: Infinity, rate: 0.37 },
      ],
    },
    ltcg: {
      single: [{ upTo: 49_450, rate: 0.0 }, { upTo: 545_500, rate: 0.15 }, { upTo: Infinity, rate: 0.20 }],
      mfj: [{ upTo: 98_900, rate: 0.0 }, { upTo: 613_700, rate: 0.15 }, { upTo: Infinity, rate: 0.20 }],
      mfs: [{ upTo: 49_450, rate: 0.0 }, { upTo: 306_850, rate: 0.15 }, { upTo: Infinity, rate: 0.20 }],
      hoh: [{ upTo: 66_200, rate: 0.0 }, { upTo: 579_600, rate: 0.15 }, { upTo: Infinity, rate: 0.20 }],
    },
    niitThreshold: NIIT_THRESHOLD,
  },
};

export const SUPPORTED_TAX_YEARS: number[] =
  Object.keys(TAX_CONSTANTS).map(Number).sort((a, b) => a - b);
export const MIN_TAX_YEAR = SUPPORTED_TAX_YEARS[0];
export const MAX_TAX_YEAR = SUPPORTED_TAX_YEARS[SUPPORTED_TAX_YEARS.length - 1];

// Returns the constants for `year` plus the year they actually came from. Future
// years clamp to the latest table (their inflation adjustments are unknowable);
// callers should surface that substitution rather than pass it off as exact.
// Years *before* the table are not clamped silently — see `handleTax`, which
// rejects them, because pre-table brackets differ enough to give wrong answers.
export function constantsFor(year: number): { constants: YearConstants; year: number } {
  const clamped = Math.min(Math.max(year, MIN_TAX_YEAR), MAX_TAX_YEAR);
  return { constants: TAX_CONSTANTS[clamped], year: clamped };
}

export interface TaxInputs {
  filing_status: Filing;
  w2_income: number;
  interest_income: number;
  ordinary_dividends: number;
  qualified_dividends: number; // subset of ordinary_dividends
  st_capital_gains: number;
  lt_capital_gains: number;
  rental_income: number;
  deduction_choice: 'standard' | 'itemized';
  itemized_deductions: number;
  carryforward_st_loss: number; // positive number = a loss carried in
  carryforward_lt_loss: number;
}

// Tax on the income interval [lo, hi] walked across `brackets`.
export function tieredTaxOnInterval(lo: number, hi: number, brackets: Bracket[]): number {
  let tax = 0;
  let prev = 0;
  for (const b of brackets) {
    const bandLo = Math.max(lo, prev);
    const bandHi = Math.min(hi, b.upTo);
    if (bandHi > bandLo) tax += (bandHi - bandLo) * b.rate;
    prev = b.upTo;
    if (prev >= hi) break;
  }
  return tax;
}

export function progressiveTax(taxable: number, brackets: Bracket[]): number {
  return tieredTaxOnInterval(0, Math.max(0, taxable), brackets);
}

// §1222 netting: carryforward losses keep their character, so net ST↔ST and
// LT↔LT first, then cross-net a loss in one bucket against a gain in the other.
// §1211(b): any remaining net loss deducts against ordinary income, capped at
// $3,000 ($1,500 MFS).
export function netCapitalGains(inp: TaxInputs): {
  ordinaryStComponent: number;
  ltComponent: number;
  ordinaryLossDeduction: number;
} {
  let netSt = inp.st_capital_gains - inp.carryforward_st_loss;
  let netLt = inp.lt_capital_gains - inp.carryforward_lt_loss;

  if (netSt < 0 && netLt > 0) {
    const use = Math.min(-netSt, netLt);
    netSt += use;
    netLt -= use;
  } else if (netLt < 0 && netSt > 0) {
    const use = Math.min(-netLt, netSt);
    netLt += use;
    netSt -= use;
  }

  const lossDeductionCap = inp.filing_status === 'mfs' ? 1_500 : 3_000;
  const totalNet = netSt + netLt;
  const ordinaryLossDeduction = totalNet < 0 ? Math.min(lossDeductionCap, -totalNet) : 0;

  return {
    ordinaryStComponent: Math.max(0, netSt),
    ltComponent: Math.max(0, netLt),
    ordinaryLossDeduction,
  };
}

export interface TaxDetail {
  agi: number;               // may be negative when a capital loss exceeds income
  deduction: number;
  taxableIncome: number;
  ordinaryTaxable: number;   // QDCG worksheet line 5
  ltTaxable: number;         // portion of taxable income at preferential rates
  capitalLossDeduction: number;
  ordinaryTax: number;
  ltcgTax: number;
  niit: number;
  total: number;
  constantsYear: number;     // year whose tables were used (see `constantsFor`)
}

export function computeFederalTaxDetail(inp: TaxInputs, year: number): TaxDetail {
  const { constants: c, year: constantsYear } = constantsFor(year);
  const fs = inp.filing_status;
  const cap = netCapitalGains(inp);

  const qualDiv = Math.min(inp.qualified_dividends, inp.ordinary_dividends);
  const nonQualDiv = Math.max(0, inp.ordinary_dividends - qualDiv);

  // Ordinary-rate slice of AGI. Left unfloored on purpose: the §1211(b) loss
  // deduction (or a rental loss) can exceed ordinary income, and that excess has
  // to reduce the preferential-rate stack rather than vanish.
  const ordinaryAgi = inp.w2_income + inp.interest_income + nonQualDiv +
    cap.ordinaryStComponent + inp.rental_income - cap.ordinaryLossDeduction;

  // §1(h) preferential stack: net capital gain + qualified dividends. Note a net
  // capital loss does not reach qualified dividends — they stay taxed at 0/15/20.
  const ltStack = cap.ltComponent + qualDiv;
  const agi = ordinaryAgi + ltStack;

  // §63: itemizing never yields less than the standard deduction. (The §63(c)(6)
  // trap — MFS where the spouse itemizes — is not modelled; we don't know the
  // spouse's return.)
  const deduction = inp.deduction_choice === 'itemized'
    ? Math.max(c.stdDeduction[fs], Math.max(0, inp.itemized_deductions))
    : c.stdDeduction[fs];

  // Qualified Dividends and Capital Gain Tax Worksheet, lines 1/5/10: the
  // deduction is absorbed by ordinary income first, and any excess eats into the
  // preferential stack.
  const taxableIncome = Math.max(0, agi - deduction);
  const ordinaryTaxable = Math.max(0, taxableIncome - ltStack);
  const ltTaxable = Math.min(taxableIncome, ltStack);

  const ordinaryTax = progressiveTax(ordinaryTaxable, c.ordinary[fs]);
  // Preferential rates are stacked: the 0/15/20% breakpoints are measured
  // against total taxable income, so walk the interval sitting above ordinary
  // taxable income.
  const ltcgTax = tieredTaxOnInterval(ordinaryTaxable, ordinaryTaxable + ltTaxable, c.ltcg[fs]);

  // §1411: 3.8% of the lesser of net investment income and (MAGI − threshold).
  // Reg. §1.1411-4(f)(4) makes the §1211(b) capital-loss deduction a properly
  // allocable deduction, so it reduces NII too; NII itself floors at zero.
  // Rental income is treated as investment income here (an approximation — §1411
  // excludes income from a trade or business the taxpayer materially
  // participates in). No MAGI add-backs (§911 exclusions) are modelled.
  const nii = Math.max(0, inp.interest_income + inp.ordinary_dividends +
    cap.ordinaryStComponent + cap.ltComponent + inp.rental_income -
    cap.ordinaryLossDeduction);
  const niit = NIIT_RATE * Math.max(0, Math.min(nii, agi - c.niitThreshold[fs]));

  return {
    agi,
    deduction,
    taxableIncome,
    ordinaryTaxable,
    ltTaxable,
    capitalLossDeduction: cap.ordinaryLossDeduction,
    ordinaryTax,
    ltcgTax,
    niit,
    total: ordinaryTax + ltcgTax + niit,
    constantsYear,
  };
}

export function computeFederalTax(inp: TaxInputs, year: number): number {
  return computeFederalTaxDetail(inp, year).total;
}

// Marginal tax incurred by realizing an incremental ST/LT gain on top of the
// baseline profile: tax(baseline + gains) − tax(baseline).
//
// Each call is independent, so marginal taxes for several trades do NOT sum to
// the tax of doing all of them: brackets are non-linear and each call gets the
// full benefit of the same carryforward losses. Callers wanting the joint number
// must pass the summed gains in one call.
export function marginalTradeTax(
  baseline: TaxInputs,
  gains: { st_gain: number; lt_gain: number },
  year: number,
  baselineTax: number,
): number {
  const withTrade: TaxInputs = {
    ...baseline,
    st_capital_gains: baseline.st_capital_gains + gains.st_gain,
    lt_capital_gains: baseline.lt_capital_gains + gains.lt_gain,
  };
  return computeFederalTax(withTrade, year) - baselineTax;
}

// ── Input validation ──────────────────────────────────────────────────────────

const FILING_STATUSES: readonly Filing[] = ['single', 'mfj', 'mfs', 'hoh'];

const NUMERIC_FIELDS = [
  'w2_income', 'interest_income', 'ordinary_dividends', 'qualified_dividends',
  'st_capital_gains', 'lt_capital_gains', 'rental_income', 'itemized_deductions',
  'carryforward_st_loss', 'carryforward_lt_loss',
] as const;

// Fields where a negative value is meaningless and would corrupt the result —
// notably the carryforward losses, which are stored as positive magnitudes and
// would otherwise be *added* to gains.
const NON_NEGATIVE_FIELDS: ReadonlySet<string> = new Set([
  'w2_income', 'interest_income', 'ordinary_dividends', 'qualified_dividends',
  'itemized_deductions', 'carryforward_st_loss', 'carryforward_lt_loss',
]);

// Coerces a stored profile into `TaxInputs`, or returns null if it can't be
// trusted. Without this a malformed row (bad filing status, a string amount)
// either throws on an undefined bracket table or silently yields NaN, which
// `JSON.stringify` turns into `null` and the client fails to deserialize.
export function sanitizeTaxInputs(raw: unknown): TaxInputs | null {
  if (!raw || typeof raw !== 'object') return null;
  const r = raw as Record<string, unknown>;

  const filing = r.filing_status as Filing;
  if (!FILING_STATUSES.includes(filing)) return null;

  const v: Record<string, number> = {};
  for (const f of NUMERIC_FIELDS) {
    const val = r[f];
    const n = val === undefined || val === null || val === '' ? 0 : Number(val);
    if (!Number.isFinite(n)) return null;
    v[f] = NON_NEGATIVE_FIELDS.has(f) ? Math.max(0, n) : n;
  }

  return {
    filing_status: filing,
    deduction_choice: r.deduction_choice === 'itemized' ? 'itemized' : 'standard',
    w2_income: v.w2_income,
    interest_income: v.interest_income,
    ordinary_dividends: v.ordinary_dividends,
    qualified_dividends: v.qualified_dividends,
    st_capital_gains: v.st_capital_gains,
    lt_capital_gains: v.lt_capital_gains,
    rental_income: v.rental_income,
    itemized_deductions: v.itemized_deductions,
    carryforward_st_loss: v.carryforward_st_loss,
    carryforward_lt_loss: v.carryforward_lt_loss,
  };
}
