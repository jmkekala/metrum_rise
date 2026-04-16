#!/usr/bin/env python3
"""
Demand system phase simulation for Metrum Rise.

Models the key demand signals and building growth from 0 to a target
population, using the actual formulas from the Rust simulation. Useful
for tuning thresholds and spotting structural problems before running
the full game.

Usage:
    python3 tools/simulate_demand.py [--target 100000] [--days 100000]

All parameters mirror the TOML tuning files so changes there should be
reflected here too.
"""

import argparse
import math


# ── Parameters (mirror demand/growth_profiles.toml + economy/profiles.toml) ──

SATURATION = 500          # signal_normalization.resident_presence_saturation_residents

# Spawn / despawn thresholds (growth_profiles.toml)
# Residential thresholds are on the net-migration scale (0.5 = equilibrium).
R_SPAWN   = 0.55;  R_DESPAWN = 0.45  # symmetric: ±10% net migration imbalance
C_SPAWN   = 0.07;  C_DESPAWN = 0.01
I_SPAWN   = 0.15;  I_DESPAWN = 0.03

# Batch fractions (action_budget.spawn_batch_fraction_by_use)
R_BATCH = 0.50;  C_BATCH = 0.40;  I_BATCH = 0.40

# Admission / removal
MAX_HH_PER_DAY      = 48    # action_budget.max_households_per_day
ADMISSION_THRESHOLD = 0.10  # household_action.admission_threshold (lowered 2026-04)
REMOVAL_THRESHOLD   = 0.55  # household_action.removal_threshold (raised 2026-04 to stop pre-supply-chain churn)

# Building economics (economy/profiles.toml)
HH_SIZE_AVG  = 3.5    # average household member count (mix of 2 and 5 seen in logs)
COM_COVERAGE = 40.0   # residents covered per commercial building (units/day capacity)
COM_WORKERS  = 5      # workers per commercial building
IND_WORKERS  = 5      # workers per industrial building

# OWA fallback: commercial can import inputs from OWA, providing this fraction
# of stock stability even without local industrial supply.
OWA_STOCK_FLOOR = 0.5


# ── Helpers ───────────────────────────────────────────────────────────────────

def clamp01(x: float) -> float:
    return max(0.0, min(1.0, x))


def norm_pos(pressure: float, threshold: float) -> float:
    """normalized_positive_pressure — matches the Rust function exactly."""
    if threshold >= 1.0:
        return 0.0
    return clamp01((pressure - threshold) / (1.0 - threshold))


# ── Candidate model ───────────────────────────────────────────────────────────
# Candidates grow with the city, representing the player zoning land slightly
# ahead of current development. Floors match what the early-game logs show.

def r_candidates(res_bld: int) -> int:
    return max(15, int(res_bld * 0.10) + 10)

def c_candidates(com_bld: int) -> int:
    return max(8,  int(com_bld * 0.15) + 5)

def i_candidates(ind_bld: int) -> int:
    return max(9,  int(ind_bld * 0.20) + 5)


# ── Main simulation ───────────────────────────────────────────────────────────

def run(target_residents: int = 100_000, max_days: int = 100_000) -> None:
    res_bld = 0;  com_bld = 0;  ind_bld = 0
    households = 0.0;  residents = 0.0
    res_cr = 0.0;  com_cr = 0.0;  ind_cr = 0.0;  adm_cr = 0.0

    header = (
        f"{'Day':>6} {'Residents':>10} {'Rbld':>7} {'Cbld':>6} {'Ibld':>5} "
        f"{'Pres':>5} {'GShrt':>6} {'Afrd':>5} "
        f"{'R_dem':>6} {'C_dem':>6} {'I_dem':>6}  Note"
    )
    print(header)
    print("─" * len(header))

    milestones = [100, 500, 1_000, 5_000, 10_000, 50_000, 100_000]
    milestones = [m for m in milestones if m <= target_residents]
    print_days = {1, 5, 10, 20, 30, 50, 100, 150, 200, 300, 500, 750,
                  1_000, 2_000, 5_000, 10_000, 20_000}

    def row(day: int, note: str = "") -> None:
        print(
            f"{day:>6} {int(residents):>10,} {res_bld:>7,} {com_bld:>6,} {ind_bld:>5,} "
            f"{pres:>5.2f} {goods_short:>6.2f} {afford:>5.2f} "
            f"{r_dem:>6.3f} {c_dem:>6.3f} {i_dem:>6.3f}  {note}"
        )

    for day in range(1, max_days + 1):
        # ── Signals ──────────────────────────────────────────────────────
        pres    = clamp01(residents / SATURATION)
        slots   = res_bld
        h_avail = clamp01((slots - households) / max(1, slots))
        h_short = 1.0 - h_avail

        # Stock stability: local commercial covers demand; industrial improves
        # quality; OWA provides a floor so commercial can operate without farms.
        if residents > 0 and com_bld > 0:
            raw_cov   = com_bld * COM_COVERAGE / residents
            ind_qual  = clamp01(ind_bld / max(1, com_bld) * 2.0)
            stock_stab = clamp01(raw_cov * (OWA_STOCK_FLOOR + (1.0 - OWA_STOCK_FLOOR) * ind_qual))
        else:
            stock_stab = 0.0
        goods_short = 1.0 - stock_stab

        # Commercial input deficit: fraction of commercial buildings without a
        # local industrial (farm) supplier. Drives industrial demand independently
        # of household goods_shortage (which OWA suppresses). Mirrors demand.rs.
        if com_bld == 0:
            com_input_deficit = 0.0
        else:
            com_input_deficit = clamp01(1.0 - ind_bld / com_bld)

        jobs      = (com_bld + ind_bld) * COM_WORKERS
        job_avail = clamp01(jobs / max(1, residents))
        ub_ramp   = clamp01(day / 10.0)
        afford    = clamp01(ub_ramp * 0.8 + job_avail * 0.2)

        # ── Admission pressure (fills existing vacancies) ─────────────────
        # Uses h_avail so it is high when slots exist to fill.
        adm_cr += norm_pos(clamp01(h_avail), ADMISSION_THRESHOLD) * MAX_HH_PER_DAY
        new_hh  = max(0, min(int(adm_cr), int(slots - households)))
        adm_cr -= new_hh
        households += new_hh
        residents  += new_hh * HH_SIZE_AVG

        # ── Removal / residential demand ──────────────────────────────────
        # removal_pressure is shared between household emigration and ResidentialGrowth.
        unhoused_ratio   = 0.0  # simplified: all housed in this model
        removal_pressure = clamp01(
            unhoused_ratio * 0.50
            + (1.0 - job_avail) * 0.25
            + goods_short * 0.25
        )
        if removal_pressure > REMOVAL_THRESHOLD and households > 0:
            remove_frac = norm_pos(removal_pressure, REMOVAL_THRESHOLD)
            rem_hh = max(0, min(int(remove_frac * MAX_HH_PER_DAY), int(households)))
            households -= rem_hh
            residents  -= rem_hh * HH_SIZE_AVG
            residents   = max(0.0, residents)

        # Residential demand: net migration balance rescaled to 0..1 (0.5 = equilibrium).
        # inflow_desire uses h_short (not h_avail) to measure unmet demand for new slots.
        # ext_conn and utility_stab both assumed 1.0 in this model.
        inflow_desire = clamp01(h_short)          # base_inflow=1, ext_conn=1, utility=1
        net_res       = max(-1.0, min(1.0, inflow_desire - removal_pressure))
        r_dem         = net_res * 0.5 + 0.5      # 0.5 = equilibrium

        # Commercial: residents with money need goods.
        c_dem = clamp01(pres * goods_short * afford)
        # Industrial: driven by commercial supply-chain gap, not goods_shortage.
        i_dem = clamp01(pres * com_input_deficit)

        # ── Spawn: Residential ────────────────────────────────────────────
        rc      = r_candidates(res_bld)
        res_cr += norm_pos(r_dem, R_SPAWN) * rc * R_BATCH
        new_r   = min(int(res_cr), rc);  res_cr -= new_r;  res_bld += new_r

        # ── Spawn: Commercial ─────────────────────────────────────────────
        cc      = c_candidates(com_bld)
        com_cr += norm_pos(c_dem, C_SPAWN) * cc * C_BATCH * pres
        new_c   = min(int(com_cr), cc);  com_cr -= new_c;  com_bld += new_c

        # ── Spawn: Industrial ─────────────────────────────────────────────
        ic      = i_candidates(ind_bld)
        ind_cr += norm_pos(i_dem, I_SPAWN) * ic * I_BATCH * pres
        new_i   = min(int(ind_cr), ic);  ind_cr -= new_i;  ind_bld += new_i

        # ── Output ────────────────────────────────────────────────────────
        if day in print_days:
            row(day)

        if milestones and residents >= milestones[0]:
            m = milestones.pop(0)
            row(day, f"← {m:,} residents")
            if residents >= target_residents or not milestones:
                break

    print()
    print("─" * 70)
    total_jobs = (com_bld + ind_bld) * COM_WORKERS
    print(f"End state: {int(residents):,} residents")
    print(f"  Buildings: {res_bld:,} residential | {com_bld:,} commercial | {ind_bld:,} industrial")
    if com_bld > 0:
        print(f"  Ratios:    1 com per {res_bld // max(1, com_bld)} res-bldg "
              f"| 1 ind per {com_bld // max(1, ind_bld)} com-bldg")
    print(f"  Jobs:      {total_jobs:,} for {int(residents):,} residents "
          f"→ job_avail={clamp01(total_jobs / max(1, residents)):.2f}")
    print(f"  Presence:  {clamp01(residents / SATURATION):.2f} "
          f"(saturates at {SATURATION} residents)")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Metrum Rise demand phase simulation")
    parser.add_argument("--target", type=int, default=100_000,
                        help="Stop when residents reach this number (default: 100000)")
    parser.add_argument("--days", type=int, default=100_000,
                        help="Hard day limit (default: 100000)")
    args = parser.parse_args()
    run(target_residents=args.target, max_days=args.days)
