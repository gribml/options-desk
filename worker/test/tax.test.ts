import { describe, expect, it } from 'vitest';
import {
  computeFederalTax,
  computeFederalTaxDetail,
  constantsFor,
  marginalTradeTax,
  netCapitalGains,
  sanitizeTaxInputs,
  tieredTaxOnInterval,
  MAX_TAX_YEAR,
  MIN_TAX_YEAR,
  NIIT_THRESHOLD,
  SUPPORTED_TAX_YEARS,
  TAX_CONSTANTS,
  type Filing,
  type TaxInputs,
} from '../src/tax';

// Every expected figure below is computed by hand from the published rate
// schedules (Rev. Proc. 2024-40 for 2025, Rev. Proc. 2025-32 for 2026) and the
// Form 1040 "Qualified Dividends and Capital Gain Tax Worksheet", so a failure
// means the engine drifted from the statute rather than from itself.

function profile(over: Partial<TaxInputs> = {}): TaxInputs {
  return {
    filing_status: 'single',
    w2_income: 0,
    interest_income: 0,
    ordinary_dividends: 0,
    qualified_dividends: 0,
    st_capital_gains: 0,
    lt_capital_gains: 0,
    rental_income: 0,
    deduction_choice: 'standard',
    itemized_deductions: 0,
    carryforward_st_loss: 0,
    carryforward_lt_loss: 0,
    ...over,
  };
}

const money = (actual: number, expected: number) => expect(actual).toBeCloseTo(expected, 2);

// ── Bracket-table integrity ───────────────────────────────────────────────────

describe('rate tables', () => {
  const statuses: Filing[] = ['single', 'mfj', 'mfs', 'hoh'];

  for (const year of SUPPORTED_TAX_YEARS) {
    for (const fs of statuses) {
      it(`${year} ${fs}: brackets ascend and the top band is unbounded`, () => {
        for (const table of [TAX_CONSTANTS[year].ordinary[fs], TAX_CONSTANTS[year].ltcg[fs]]) {
          expect(table.length).toBeGreaterThan(1);
          expect(table[table.length - 1].upTo).toBe(Infinity);
          for (let i = 1; i < table.length; i++) {
            expect(table[i].upTo).toBeGreaterThan(table[i - 1].upTo);
            expect(table[i].rate).toBeGreaterThan(table[i - 1].rate);
          }
        }
      });

      it(`${year} ${fs}: statutory top rates`, () => {
        const { ordinary, ltcg } = TAX_CONSTANTS[year];
        expect(ordinary[fs][0].rate).toBe(0.10);
        expect(ordinary[fs][ordinary[fs].length - 1].rate).toBe(0.37);
        expect(ltcg[fs][0].rate).toBe(0.0);
        expect(ltcg[fs][ltcg[fs].length - 1].rate).toBe(0.20);
        expect(ltcg[fs].map((b) => b.rate)).toEqual([0.0, 0.15, 0.20]);
      });
    }

    it(`${year}: §1(j) MFS ordinary brackets are half the MFJ brackets`, () => {
      const { ordinary } = TAX_CONSTANTS[year];
      ordinary.mfs.forEach((b, i) => {
        if (b.upTo === Infinity) return;
        expect(b.upTo).toBe(ordinary.mfj[i].upTo / 2);
      });
    });

    it(`${year}: §63(c) MFJ standard deduction is twice single, MFS equals single`, () => {
      const d = TAX_CONSTANTS[year].stdDeduction;
      expect(d.mfj).toBe(d.single * 2);
      expect(d.mfs).toBe(d.single);
      expect(d.hoh).toBeGreaterThan(d.single);
      expect(d.hoh).toBeLessThan(d.mfj);
    });
  }

  it('2025 and 2026 standard deductions match the published amounts', () => {
    // 2025 reflects OBBBA (P.L. 119-21 §70102), not the original Rev. Proc. figure.
    expect(TAX_CONSTANTS[2025].stdDeduction)
      .toEqual({ single: 15_750, mfj: 31_500, mfs: 15_750, hoh: 23_625 });
    expect(TAX_CONSTANTS[2026].stdDeduction)
      .toEqual({ single: 16_100, mfj: 32_200, mfs: 16_100, hoh: 24_150 });
  });

  it('§1411(b) NIIT thresholds are the statutory, non-indexed amounts', () => {
    expect(NIIT_THRESHOLD).toEqual({ single: 200_000, mfj: 250_000, mfs: 125_000, hoh: 200_000 });
    for (const year of SUPPORTED_TAX_YEARS) {
      expect(TAX_CONSTANTS[year].niitThreshold).toEqual(NIIT_THRESHOLD);
    }
  });

  it('future years fall back to the latest table and report which year was used', () => {
    expect(constantsFor(MAX_TAX_YEAR + 5).year).toBe(MAX_TAX_YEAR);
    expect(constantsFor(MAX_TAX_YEAR + 5).constants).toBe(TAX_CONSTANTS[MAX_TAX_YEAR]);
    expect(constantsFor(MIN_TAX_YEAR).year).toBe(MIN_TAX_YEAR);
    // Past years also clamp here; `handleTax` rejects them instead of using this.
    expect(constantsFor(MIN_TAX_YEAR - 1).year).toBe(MIN_TAX_YEAR);
  });
});

describe('tieredTaxOnInterval', () => {
  const brackets = [{ upTo: 100, rate: 0.1 }, { upTo: 200, rate: 0.2 }, { upTo: Infinity, rate: 0.3 }];

  it('walks a full interval from zero', () => {
    money(tieredTaxOnInterval(0, 250, brackets), 100 * 0.1 + 100 * 0.2 + 50 * 0.3);
  });

  it('walks an interval that starts mid-table', () => {
    money(tieredTaxOnInterval(150, 250, brackets), 50 * 0.2 + 50 * 0.3);
  });

  it('is zero for an empty or inverted interval', () => {
    money(tieredTaxOnInterval(150, 150, brackets), 0);
    money(tieredTaxOnInterval(200, 100, brackets), 0);
  });
});

// ── Ordinary income (§1 rate schedules) ───────────────────────────────────────

describe('ordinary income', () => {
  it('single, $100k wages, 2025', () => {
    // Taxable 84,250 = 100,000 − 15,750.
    // 11,925@10 + 36,550@12 + 35,775@22 = 1,192.50 + 4,386.00 + 7,870.50
    money(computeFederalTax(profile({ w2_income: 100_000 }), 2025), 13_449);
  });

  it('single, $100k wages, 2026 (indexed brackets differ from 2025)', () => {
    // Taxable 83,900 = 100,000 − 16,100.
    // 12,400@10 + 38,000@12 + 33,500@22 = 1,240 + 4,560 + 7,370
    money(computeFederalTax(profile({ w2_income: 100_000 }), 2026), 13_170);
  });

  it('married filing jointly, $200k wages, 2025', () => {
    // Taxable 168,500. 23,850@10 + 73,100@12 + 71,550@22
    money(computeFederalTax(profile({ filing_status: 'mfj', w2_income: 200_000 }), 2025), 26_898);
  });

  it('head of household, $80k wages, 2025', () => {
    // Taxable 56,375. 17,000@10 + 39,375@12
    money(computeFederalTax(profile({ filing_status: 'hoh', w2_income: 80_000 }), 2025), 6_425);
  });

  it('income under the standard deduction owes nothing', () => {
    money(computeFederalTax(profile({ w2_income: 10_000 }), 2025), 0);
  });

  it('single, $1M wages, 2025 reaches the 37% band', () => {
    // Taxable 984,250: 1,192.50 + 4,386 + 12,072.50 + 22,548 + 17,032
    //                  + 375,825@35 (131,538.75) + 357,900@37 (132,423)
    money(computeFederalTax(profile({ w2_income: 1_000_000 }), 2025), 321_192.75);
  });

  it('interest and non-qualified dividends are taxed as ordinary income', () => {
    const split = profile({ w2_income: 60_000, interest_income: 25_000, ordinary_dividends: 15_000 });
    money(computeFederalTax(split, 2025), computeFederalTax(profile({ w2_income: 100_000 }), 2025));
  });
});

// ── §1(h) preferential rates, stacked on ordinary income ─────────────────────

describe('long-term gains and qualified dividends', () => {
  it('stacks a long-term gain on top of ordinary taxable income', () => {
    // Ordinary taxable 34,250 → 3,871.50. Stack 20,000 over it: 14,100 fills the
    // 0% band (ends at 48,350), the remaining 5,900 is taxed at 15% → 885.
    const d = computeFederalTaxDetail(profile({ w2_income: 50_000, lt_capital_gains: 20_000 }), 2025);
    money(d.ordinaryTaxable, 34_250);
    money(d.ltTaxable, 20_000);
    money(d.ordinaryTax, 3_871.50);
    money(d.ltcgTax, 885);
    money(d.total, 4_756.50);
  });

  it('taxes a gain entirely inside the 0% band at nothing', () => {
    money(computeFederalTax(profile({ lt_capital_gains: 40_000 }), 2025), 0);
  });

  it('applies the deduction to ordinary income first, then to the preferential stack', () => {
    // QDCG worksheet line 5 = max(0, taxable income − stack). With 10,000 of
    // wages and a 15,750 deduction, 5,750 of deduction spills onto the stack:
    // taxable income 44,250, all of it preferential and all in the 0% band.
    const d = computeFederalTaxDetail(profile({ w2_income: 10_000, lt_capital_gains: 50_000 }), 2025);
    money(d.taxableIncome, 44_250);
    money(d.ordinaryTaxable, 0);
    money(d.ltTaxable, 44_250);
    money(d.total, 0);
  });

  it('ordinary taxable + preferential taxable always equals taxable income', () => {
    for (const [w2, lt] of [[0, 0], [10_000, 50_000], [50_000, 20_000], [500_000, 200_000]]) {
      const d = computeFederalTaxDetail(profile({ w2_income: w2, lt_capital_gains: lt }), 2025);
      money(d.ordinaryTaxable + d.ltTaxable, d.taxableIncome);
    }
  });

  it('reaches the 20% band and charges NIIT on the gain', () => {
    // Ordinary taxable 484,250 → 139,034.75.
    // Stack: 49,150 to the 533,400 breakpoint @15% (7,372.50) + 150,850 @20% (30,170).
    // NIIT: min(NII 200,000, MAGI 700,000 − 200,000) × 3.8% = 7,600.
    const d = computeFederalTaxDetail(profile({ w2_income: 500_000, lt_capital_gains: 200_000 }), 2025);
    money(d.ordinaryTax, 139_034.75);
    money(d.ltcgTax, 37_542.50);
    money(d.niit, 7_600);
    money(d.total, 184_177.25);
  });

  it('taxes qualified dividends at preferential rates and the rest as ordinary', () => {
    // 2,000 non-qualified → ordinary taxable 46,250 (5,311.50).
    // 8,000 qualified stacked: 2,100 @0% + 5,900 @15% = 885.
    money(computeFederalTax(
      profile({ w2_income: 60_000, ordinary_dividends: 10_000, qualified_dividends: 8_000 }),
      2025,
    ), 6_196.50);
  });

  it('clamps qualified dividends to the ordinary dividend total', () => {
    const overstated = profile({ w2_income: 60_000, ordinary_dividends: 10_000, qualified_dividends: 20_000 });
    const clamped = profile({ w2_income: 60_000, ordinary_dividends: 10_000, qualified_dividends: 10_000 });
    money(computeFederalTax(overstated, 2025), computeFederalTax(clamped, 2025));
    money(computeFederalTax(clamped, 2025), 5_956.50);
  });
});

// ── §1211(b) / §1222 capital-loss netting ────────────────────────────────────

describe('capital losses', () => {
  it('deducts at most $3,000 of net capital loss against ordinary income', () => {
    // 10,000 loss, only 3,000 usable: ordinary income 97,000, taxable 81,250.
    const d = computeFederalTaxDetail(profile({ w2_income: 100_000, st_capital_gains: -10_000 }), 2025);
    money(d.capitalLossDeduction, 3_000);
    money(d.total, 12_789);
  });

  it('caps the loss deduction at $1,500 for married filing separately', () => {
    const d = computeFederalTaxDetail(
      profile({ filing_status: 'mfs', w2_income: 100_000, st_capital_gains: -10_000 }),
      2025,
    );
    money(d.capitalLossDeduction, 1_500);
    money(d.total, 13_119);
  });

  it('cross-nets a short-term loss against a long-term gain before the $3,000 cap', () => {
    // −10,000 ST against +4,000 LT leaves a 6,000 net loss → 3,000 deductible.
    const d = computeFederalTaxDetail(
      profile({ w2_income: 100_000, st_capital_gains: -10_000, lt_capital_gains: 4_000 }),
      2025,
    );
    money(d.ltTaxable, 0);
    money(d.total, 12_789);
  });

  it('cross-nets a long-term loss against a short-term gain', () => {
    // +10,000 ST against −4,000 LT → 6,000 net short-term gain, taxed as ordinary.
    const d = computeFederalTaxDetail(
      profile({ w2_income: 100_000, st_capital_gains: 10_000, lt_capital_gains: -4_000 }),
      2025,
    );
    money(d.capitalLossDeduction, 0);
    money(d.ltTaxable, 0);
    money(d.total, 14_769);
  });

  it('§1212(b): a carried-forward loss keeps its character and absorbs same-year gains', () => {
    // A 10,000 short-term carryforward fully absorbs a 10,000 long-term gain, so
    // the result matches having realized no gain at all.
    const withGain = profile({
      w2_income: 100_000,
      lt_capital_gains: 10_000,
      carryforward_st_loss: 10_000,
    });
    money(computeFederalTax(withGain, 2025), computeFederalTax(profile({ w2_income: 100_000 }), 2025));
  });

  it('a loss exceeding ordinary income reduces the preferential stack, not just AGI', () => {
    // 100,000 qualified dividends, no wages, 20,000 ST loss. The 3,000 deduction
    // drives the ordinary slice of AGI to −3,000, and that has to come off the
    // preferential stack: taxable income 81,250, all preferential.
    //   48,350 @0% + 32,900 @15% = 4,935. (Dropping the negative ordinary slice
    //   would tax 84,250 and overstate the bill by 450.)
    const d = computeFederalTaxDetail(profile({
      ordinary_dividends: 100_000,
      qualified_dividends: 100_000,
      st_capital_gains: -20_000,
    }), 2025);
    money(d.agi, 97_000);
    money(d.taxableIncome, 81_250);
    money(d.ordinaryTaxable, 0);
    money(d.ltTaxable, 81_250);
    money(d.total, 4_935);
  });

  it('does not let a net capital loss reach qualified dividends', () => {
    // Qualified dividends stay at preferential rates no matter how large the
    // capital loss: ordinary taxable 81,250 (12,789) + 50,000 @15% (7,500).
    const d = computeFederalTaxDetail(profile({
      w2_income: 100_000,
      ordinary_dividends: 50_000,
      qualified_dividends: 50_000,
      st_capital_gains: -100_000,
    }), 2025);
    money(d.ordinaryTax, 12_789);
    money(d.ltcgTax, 7_500);
    money(d.total, 20_289);
  });

  it('nets by character before cross-netting', () => {
    const cap = netCapitalGains(profile({
      st_capital_gains: 5_000,
      carryforward_st_loss: 12_000,
      lt_capital_gains: 20_000,
      carryforward_lt_loss: 4_000,
    }));
    // ST: 5,000 − 12,000 = −7,000. LT: 20,000 − 4,000 = 16,000.
    // Cross-net → ST 0, LT 9,000, no ordinary deduction.
    money(cap.ordinaryStComponent, 0);
    money(cap.ltComponent, 9_000);
    money(cap.ordinaryLossDeduction, 0);
  });
});

// ── §1411 Net Investment Income Tax ──────────────────────────────────────────

describe('NIIT', () => {
  it('charges 3.8% when NII and the MAGI excess are equal', () => {
    // Ordinary taxable 184,250 (37,067) + 100,000 @15% (15,000)
    // + 3.8% × min(100,000, 300,000 − 200,000) = 3,800.
    const d = computeFederalTaxDetail(profile({ w2_income: 200_000, lt_capital_gains: 100_000 }), 2025);
    money(d.niit, 3_800);
    money(d.total, 55_867);
  });

  it('is not charged on wages — only on investment income', () => {
    const d = computeFederalTaxDetail(profile({ w2_income: 400_000 }), 2025);
    money(d.niit, 0);
    money(d.total, 104_034.75);
  });

  it('is zero when MAGI sits exactly at the threshold', () => {
    const d = computeFederalTaxDetail(profile({ w2_income: 150_000, interest_income: 50_000 }), 2025);
    money(d.agi, 200_000);
    money(d.niit, 0);
    money(d.total, 37_067);
  });

  it('uses each filing status threshold', () => {
    for (const fs of ['single', 'mfj', 'mfs', 'hoh'] as Filing[]) {
      const thr = NIIT_THRESHOLD[fs];
      const below = computeFederalTaxDetail(
        profile({ filing_status: fs, interest_income: thr }), 2025);
      const above = computeFederalTaxDetail(
        profile({ filing_status: fs, interest_income: thr + 10_000 }), 2025);
      money(below.niit, 0);
      money(above.niit, 10_000 * 0.038);
    }
  });

  it('Reg. §1.1411-4(f)(4): the §1211(b) loss deduction reduces net investment income', () => {
    // Interest 20,000 with a 50,000 ST loss. The 3,000 allowed loss is a properly
    // allocable deduction, so NII is 17,000, not 20,000 — and here NII (not the
    // MAGI excess of 67,000) is the binding term, so it changes the bill.
    const d = computeFederalTaxDetail(
      profile({ w2_income: 250_000, interest_income: 20_000, st_capital_gains: -50_000 }), 2025);
    money(d.niit, 17_000 * 0.038); // 646, vs 760 if the deduction were ignored
    money(d.ordinaryTax, 57_484.75);
    money(d.total, 58_130.75);
  });

  it('never charges NIIT on negative net investment income', () => {
    const d = computeFederalTaxDetail(
      profile({ w2_income: 400_000, rental_income: -50_000 }), 2025);
    money(d.niit, 0);
  });
});

// ── §63 deduction choice ─────────────────────────────────────────────────────

describe('deductions', () => {
  it('uses itemized deductions when they exceed the standard deduction', () => {
    const d = computeFederalTaxDetail(
      profile({ w2_income: 100_000, deduction_choice: 'itemized', itemized_deductions: 30_000 }), 2025);
    money(d.deduction, 30_000);
    money(d.total, 10_314);
  });

  it('§63(b): never deducts less than the standard deduction', () => {
    const d = computeFederalTaxDetail(
      profile({ w2_income: 100_000, deduction_choice: 'itemized', itemized_deductions: 5_000 }), 2025);
    money(d.deduction, 15_750);
    money(d.total, 13_449);
  });

  it('ignores itemized deductions when the standard deduction is chosen', () => {
    const d = computeFederalTaxDetail(
      profile({ w2_income: 100_000, deduction_choice: 'standard', itemized_deductions: 90_000 }), 2025);
    money(d.deduction, 15_750);
  });
});

// ── Marginal tax on a trade ──────────────────────────────────────────────────

describe('marginalTradeTax', () => {
  const marginal = (base: TaxInputs, st: number, lt: number, year = 2025) =>
    marginalTradeTax(base, { st_gain: st, lt_gain: lt }, year, computeFederalTax(base, year));

  it('is zero for a zero gain', () => {
    money(marginal(profile({ w2_income: 100_000 }), 0, 0), 0);
  });

  it('charges a short-term gain at the ordinary marginal rate', () => {
    // Taxable 84,250 → 10,000 more stays inside the 22% band (top 103,350).
    money(marginal(profile({ w2_income: 100_000 }), 10_000, 0), 2_200);
  });

  it('spans ordinary brackets for a short-term gain', () => {
    // 19,100 to the 103,350 breakpoint @22% + 10,900 @24%.
    money(marginal(profile({ w2_income: 100_000 }), 30_000, 0), 4_202 + 2_616);
  });

  it('charges nothing for a long-term gain that stays in the 0% band', () => {
    // Taxable 14,250 + 20,000 of gain = 34,250, under the 48,350 breakpoint.
    money(marginal(profile({ w2_income: 30_000 }), 0, 20_000), 0);
  });

  it('straddles the 0%/15% breakpoint for a long-term gain', () => {
    // Ordinary taxable 40,000: 8,350 of gain fills the 0% band, 11,650 @15%.
    money(marginal(profile({ w2_income: 55_750 }), 0, 20_000), 1_747.50);
  });

  it('adds NIIT once the gain pushes MAGI over the threshold', () => {
    // 20,000 @15% (3,000) + 3.8% on the 10,000 of MAGI above 200,000 (380).
    money(marginal(profile({ w2_income: 190_000 }), 0, 20_000), 3_380);
  });

  it('returns a negative marginal tax for a realized loss, capped by §1211(b)', () => {
    // Only 3,000 of a 10,000 loss is usable, at the 22% marginal rate.
    money(marginal(profile({ w2_income: 100_000 }), -10_000, 0), -660);
  });

  it('is convex: the joint marginal tax is at least the sum of the separate ones', () => {
    // Why the batch endpoint's per-item taxes must not be summed — each item is
    // measured against the same baseline, so the sum understates the true total.
    for (const w2 of [0, 90_000, 240_000, 500_000]) {
      const base = profile({ w2_income: w2 });
      const a = marginal(base, 20_000, 0);
      const b = marginal(base, 0, 40_000);
      expect(marginal(base, 20_000, 40_000)).toBeGreaterThanOrEqual(a + b - 1e-6);
    }
  });

  it('understates by a real amount when a gain pushes another across a breakpoint', () => {
    // Ordinary taxable 484,250. Each gain alone also drags 3,800 of NIIT (all
    // capital gain is net investment income, short- or long-term):
    //   short-term: 100,000 @35% + 3,800            = 38,800
    //   long-term:  49,150 @15% + 50,850 @20% + 3,800 = 21,342.50
    // Together, the short-term gain lifts the entire long-term stack past the
    // 533,400 breakpoint, so all 100,000 of it is taxed at 20% and NIIT applies
    // to 200,000 of gain — 2,457.50 more than summing the two separately.
    const base = profile({ w2_income: 500_000 });
    money(marginal(base, 100_000, 0), 38_800);
    money(marginal(base, 0, 100_000), 21_342.50);
    money(marginal(base, 100_000, 100_000), 35_000 + 20_000 + 7_600);
  });
});

// ── Input validation ─────────────────────────────────────────────────────────

describe('sanitizeTaxInputs', () => {
  it('accepts a well-formed profile unchanged', () => {
    const p = profile({ w2_income: 100_000, st_capital_gains: -5_000 });
    expect(sanitizeTaxInputs(p)).toEqual(p);
  });

  it('rejects a non-object or unknown filing status', () => {
    expect(sanitizeTaxInputs(null)).toBeNull();
    expect(sanitizeTaxInputs('nope')).toBeNull();
    expect(sanitizeTaxInputs({ ...profile(), filing_status: 'married' })).toBeNull();
    expect(sanitizeTaxInputs({ ...profile(), filing_status: undefined })).toBeNull();
  });

  it('rejects non-numeric amounts instead of producing NaN', () => {
    expect(sanitizeTaxInputs({ ...profile(), w2_income: 'lots' })).toBeNull();
    expect(sanitizeTaxInputs({ ...profile(), lt_capital_gains: NaN })).toBeNull();
    expect(sanitizeTaxInputs({ ...profile(), interest_income: Infinity })).toBeNull();
  });

  it('coerces numeric strings and treats missing amounts as zero', () => {
    const s = sanitizeTaxInputs({ filing_status: 'single', w2_income: '100000' })!;
    expect(s.w2_income).toBe(100_000);
    expect(s.lt_capital_gains).toBe(0);
    expect(s.deduction_choice).toBe('standard');
  });

  it('clamps a negative carryforward loss, which would otherwise invent a gain', () => {
    const s = sanitizeTaxInputs({ ...profile(), carryforward_st_loss: -50_000 })!;
    expect(s.carryforward_st_loss).toBe(0);
    money(computeFederalTax(s, 2025), 0);
  });

  it('keeps capital gains and rental income signed', () => {
    const s = sanitizeTaxInputs({
      ...profile(), st_capital_gains: -5_000, lt_capital_gains: -1_000, rental_income: -2_000,
    })!;
    expect(s.st_capital_gains).toBe(-5_000);
    expect(s.lt_capital_gains).toBe(-1_000);
    expect(s.rental_income).toBe(-2_000);
  });

  it('falls back to the standard deduction for an unrecognized choice', () => {
    const s = sanitizeTaxInputs({ ...profile(), deduction_choice: 'bogus' })!;
    expect(s.deduction_choice).toBe('standard');
  });
});

// ── Invariants ───────────────────────────────────────────────────────────────

describe('invariants', () => {
  const statuses: Filing[] = ['single', 'mfj', 'mfs', 'hoh'];

  it('a zero profile owes zero tax in every status and year', () => {
    for (const year of SUPPORTED_TAX_YEARS) {
      for (const fs of statuses) {
        money(computeFederalTax(profile({ filing_status: fs }), year), 0);
      }
    }
  });

  it('tax is non-decreasing in wage income', () => {
    for (const fs of statuses) {
      let prev = -1;
      for (let w2 = 0; w2 <= 1_500_000; w2 += 12_500) {
        const t = computeFederalTax(profile({ filing_status: fs, w2_income: w2 }), 2025);
        expect(t).toBeGreaterThanOrEqual(prev);
        prev = t;
      }
    }
  });

  it('tax is finite and non-negative across a wide input sweep', () => {
    for (const fs of statuses) {
      for (const w2 of [0, 45_000, 200_000, 900_000]) {
        for (const lt of [-80_000, 0, 30_000, 400_000]) {
          for (const st of [-80_000, 0, 30_000]) {
            for (const cf of [0, 25_000]) {
              const t = computeFederalTax(profile({
                filing_status: fs, w2_income: w2, lt_capital_gains: lt,
                st_capital_gains: st, carryforward_st_loss: cf,
                interest_income: 5_000, ordinary_dividends: 8_000, qualified_dividends: 6_000,
              }), 2025);
              expect(Number.isFinite(t)).toBe(true);
              expect(t).toBeGreaterThanOrEqual(0);
            }
          }
        }
      }
    }
  });

  // Marginal-rate ceilings. Restricted to loss-free baselines: with a carryforward
  // in play a realized gain also burns the $3,000 ordinary loss deduction, so the
  // marginal rate on the gain can legitimately exceed these bounds.
  //
  // Deliberately not asserted: that a long-term gain is never taxed worse than a
  // short-term one. Under current law the 0% breakpoint (48,350 single, 2025)
  // sits just below the top of the 12% ordinary band (48,475), so in that narrow
  // window a marginal long-term dollar costs 15% and an ordinary one 12%.
  it('a long-term gain never costs more than 23.8% (20% + NIIT)', () => {
    for (const fs of statuses) {
      for (const w2 of [0, 60_000, 250_000, 800_000]) {
        const base = profile({ filing_status: fs, w2_income: w2, interest_income: 40_000 });
        const baseTax = computeFederalTax(base, 2025);
        for (const gain of [1_000, 50_000, 600_000]) {
          const m = marginalTradeTax(base, { st_gain: 0, lt_gain: gain }, 2025, baseTax);
          expect(m).toBeGreaterThanOrEqual(0);
          expect(m).toBeLessThanOrEqual(gain * 0.238 + 1e-6);
        }
      }
    }
  });

  it('a short-term gain never costs more than 40.8% (37% + NIIT)', () => {
    for (const fs of statuses) {
      for (const w2 of [0, 60_000, 250_000, 800_000]) {
        const base = profile({ filing_status: fs, w2_income: w2, interest_income: 40_000 });
        const baseTax = computeFederalTax(base, 2025);
        for (const gain of [1_000, 50_000, 600_000]) {
          const m = marginalTradeTax(base, { st_gain: gain, lt_gain: 0 }, 2025, baseTax);
          expect(m).toBeGreaterThanOrEqual(0);
          expect(m).toBeLessThanOrEqual(gain * 0.408 + 1e-6);
        }
      }
    }
  });
});
