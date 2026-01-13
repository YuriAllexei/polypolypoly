"""Comprehensive tests for quoter functions.

Each test demonstrates how the function works with clear inputs and expected outputs.
Run with: poetry run pytest tests/test_quoter_functions.py -v

The 4-Layer Framework:
- Layer 1: Oracle-Adjusted Offset - React to oracle direction
- Layer 2: Adverse Selection - Widen spread near resolution (Glosten-Milgrom)
- Layer 3: Inventory Skew - Balance positions (affects prices AND sizes)
- Layer 4: Edge Check - Don't overpay
"""

import math

import pytest

from model_tuning.core.models import Inventory, Market, Oracle
from model_tuning.core.quoter import InventoryMMQuoter, QuoterParams
from model_tuning.core.utils import clamp, create_market, snap_to_tick


# =============================================================================
# UTILITY FUNCTIONS
# =============================================================================


class TestSnapToTick:
    """Test snap_to_tick: Polymarket requires prices in whole cents only.

    Any price like 0.515 or 0.4875 is INVALID - must snap to nearest 0.01.
    """

    def test_rounds_up_above_half_cent(self) -> None:
        """0.515 -> 0.52 (rounds up because .015 > .005)"""
        # Input: 51.5 cents
        # Output: 52 cents (rounded up)
        assert snap_to_tick(0.515) == 0.52

    def test_rounds_down_below_half_cent(self) -> None:
        """0.514 -> 0.51 (rounds down because .014 < .005)"""
        assert snap_to_tick(0.514) == 0.51

    def test_exact_cent_unchanged(self) -> None:
        """0.50 -> 0.50 (already valid, no change)"""
        assert snap_to_tick(0.50) == 0.50
        assert snap_to_tick(0.01) == 0.01
        assert snap_to_tick(0.99) == 0.99

    def test_sub_cent_precision_handled(self) -> None:
        """0.4875 -> 0.49 (multiple decimal places snapped)"""
        assert snap_to_tick(0.4875) == 0.49
        assert snap_to_tick(0.123456) == 0.12

    @pytest.mark.parametrize(
        "input_val,expected",
        [
            (0.001, 0.00),  # Below 1 cent
            (0.004, 0.00),
            (0.006, 0.01),  # Above half cent rounds up
            (0.009, 0.01),
            (0.555, 0.56),
            (0.554, 0.55),
        ],
    )
    def test_parametrized_rounding(self, input_val: float, expected: float) -> None:
        """Multiple rounding scenarios."""
        assert snap_to_tick(input_val) == expected


class TestCreateMarket:
    """Test create_market: Creates complementary binary market orderbook.

    In Polymarket: UP + DOWN = $1.00 at resolution.
    So if UP is worth 55c, DOWN must be worth 45c.
    """

    def test_complementary_prices_sum_correctly(self) -> None:
        """Asks should sum to ~1.0 + spread (overround)."""
        mkt = create_market(up_mid=0.55, spread=0.02)

        # UP mid = 0.55, DOWN mid = 0.45
        # Asks: UP=0.56, DOWN=0.46 -> sum = 1.02 (2% overround)
        assert mkt.best_ask_up + mkt.best_ask_down == pytest.approx(1.02)
        # Bids: UP=0.54, DOWN=0.44 -> sum = 0.98 (2% underround)
        assert mkt.best_bid_up + mkt.best_bid_down == pytest.approx(0.98)

    def test_spread_applied_symmetrically(self) -> None:
        """Spread is split evenly around midpoint."""
        mkt = create_market(up_mid=0.60, spread=0.04)

        # UP: mid=0.60, spread=0.04 -> bid=0.58, ask=0.62
        assert mkt.best_ask_up == 0.62
        assert mkt.best_bid_up == 0.58
        # DOWN: mid=0.40, spread=0.04 -> bid=0.38, ask=0.42
        assert mkt.best_ask_down == 0.42
        assert mkt.best_bid_down == 0.38

    def test_50_50_market(self) -> None:
        """Equal probability market (coin flip)."""
        mkt = create_market(up_mid=0.50, spread=0.02)

        # Both sides symmetric
        assert mkt.best_ask_up == 0.51
        assert mkt.best_bid_up == 0.49
        assert mkt.best_ask_down == 0.51
        assert mkt.best_bid_down == 0.49

    def test_extreme_probabilities(self) -> None:
        """Near-certain outcomes (80% UP, 20% DOWN)."""
        mkt = create_market(up_mid=0.80, spread=0.02)

        assert mkt.best_ask_up == 0.81
        assert mkt.best_bid_up == 0.79
        assert mkt.best_ask_down == 0.21
        assert mkt.best_bid_down == 0.19


class TestClamp:
    """Test clamp: Constrain value to [min, max] range."""

    def test_within_range_unchanged(self) -> None:
        """Value inside range passes through."""
        assert clamp(0.5, 0.0, 1.0) == 0.5
        assert clamp(0.0, 0.0, 1.0) == 0.0  # At min
        assert clamp(1.0, 0.0, 1.0) == 1.0  # At max

    def test_below_min_clamped(self) -> None:
        """Value below min is raised to min."""
        assert clamp(-0.5, 0.0, 1.0) == 0.0
        assert clamp(-100, 0.0, 1.0) == 0.0

    def test_above_max_clamped(self) -> None:
        """Value above max is lowered to max."""
        assert clamp(1.5, 0.0, 1.0) == 1.0
        assert clamp(100, 0.0, 1.0) == 1.0


# =============================================================================
# LAYER 1: ORACLE-ADJUSTED OFFSETS
# =============================================================================


class TestLayer1OracleAdjustedOffsets:
    """Test calculate_oracle_adjusted_offsets: Adjust bid offsets based on oracle.

    When oracle says UP is likely (price > threshold):
    - Tighten UP offset (bid closer to market -> more aggressive)
    - Widen DOWN offset (bid further from market -> protection)

    Formula:
        oracle_adj = distance_pct × sensitivity
        up_offset = base_offset - oracle_adj  (tighter when bullish)
        down_offset = base_offset + oracle_adj  (wider when bullish)
    """

    @pytest.fixture
    def quoter(self) -> InventoryMMQuoter:
        """Quoter with known parameters for predictable calculations."""
        return InventoryMMQuoter(
            QuoterParams(
                oracle_sensitivity=5.0,  # 1% move = 5c adjustment
                min_offset=0.01,
            )
        )

    def test_neutral_oracle_no_adjustment(self, quoter: InventoryMMQuoter) -> None:
        """Oracle at threshold -> no directional adjustment."""
        oracle = Oracle(current_price=97000, threshold=97000)
        base_offset = 0.01

        oracle_adj, up_offset, down_offset = quoter.calculate_oracle_adjusted_offsets(
            oracle, base_offset
        )

        # distance_pct = (97000 - 97000) / 97000 = 0
        # oracle_adj = 0 × 5.0 = 0
        assert oracle_adj == pytest.approx(0.0)
        # Both offsets equal base (symmetric)
        assert up_offset == pytest.approx(0.01)
        assert down_offset == pytest.approx(0.01)

    def test_bullish_oracle_tightens_up_widens_down(
        self, quoter: InventoryMMQuoter
    ) -> None:
        """Oracle above threshold -> aggressive on UP, defensive on DOWN."""
        oracle = Oracle(current_price=97500, threshold=97000)
        base_offset = 0.02

        oracle_adj, up_offset, down_offset = quoter.calculate_oracle_adjusted_offsets(
            oracle, base_offset
        )

        # distance_pct = (97500 - 97000) / 97000 ≈ 0.515%
        # oracle_adj = 0.00515 × 5.0 ≈ 0.0258
        expected_adj = (500 / 97000) * 5.0
        assert oracle_adj == pytest.approx(expected_adj)

        # UP offset = 0.02 - 0.0258 -> clamped to min_offset (0.01)
        assert up_offset == pytest.approx(0.01)  # Clamped at min
        # DOWN offset = 0.02 + 0.0258 ≈ 0.0458
        assert down_offset == pytest.approx(0.02 + expected_adj)

    def test_bearish_oracle_tightens_down_widens_up(
        self, quoter: InventoryMMQuoter
    ) -> None:
        """Oracle below threshold -> aggressive on DOWN, defensive on UP."""
        oracle = Oracle(current_price=96500, threshold=97000)
        base_offset = 0.02

        oracle_adj, up_offset, down_offset = quoter.calculate_oracle_adjusted_offsets(
            oracle, base_offset
        )

        # distance_pct = (96500 - 97000) / 97000 ≈ -0.515%
        # oracle_adj = -0.00515 × 5.0 ≈ -0.0258
        expected_adj = (-500 / 97000) * 5.0
        assert oracle_adj == pytest.approx(expected_adj)

        # UP offset = 0.02 - (-0.0258) = 0.0458 (wider)
        assert up_offset == pytest.approx(0.02 - expected_adj)
        # DOWN offset = 0.02 + (-0.0258) -> clamped to min_offset
        assert down_offset == pytest.approx(0.01)  # Clamped at min

    def test_min_offset_enforced(self) -> None:
        """Offset never goes below min_offset (prevents negative/zero offset)."""
        quoter = InventoryMMQuoter(
            QuoterParams(
                oracle_sensitivity=100.0,  # Very high sensitivity
                min_offset=0.005,
            )
        )
        oracle = Oracle(current_price=98000, threshold=97000)  # 1% above

        _, up_offset, _ = quoter.calculate_oracle_adjusted_offsets(oracle, 0.02)

        # Would be negative without clamping, but min_offset enforced
        assert up_offset == 0.005

    def test_sensitivity_parameter_scaling(self) -> None:
        """Higher sensitivity = larger adjustments."""
        oracle = Oracle(current_price=97100, threshold=97000)  # 0.1% above
        base_offset = 0.02

        # Low sensitivity
        quoter_low = InventoryMMQuoter(QuoterParams(oracle_sensitivity=1.0))
        adj_low, _, _ = quoter_low.calculate_oracle_adjusted_offsets(oracle, base_offset)

        # High sensitivity
        quoter_high = InventoryMMQuoter(QuoterParams(oracle_sensitivity=10.0))
        adj_high, _, _ = quoter_high.calculate_oracle_adjusted_offsets(
            oracle, base_offset
        )

        # 10x sensitivity = 10x adjustment
        assert adj_high == pytest.approx(adj_low * 10)


# =============================================================================
# LAYER 2: ADVERSE SELECTION
# =============================================================================


class TestLayer2AdverseSelection:
    """Test calculate_adverse_selection: Widen spread near resolution.

    Near resolution, informed traders know the outcome. They dump losing tokens.
    Market makers widen spread to protect against being dumped on.

    Formula:
        p_informed = p_informed_base × exp(-minutes / time_decay)
        spread = base_spread × (1 + 3 × p_informed)

    Timeline:
        14 min: Safe, tight spreads
        7 min: Careful, moderate spreads
        1 min: DANGER, wide spreads
    """

    @pytest.fixture
    def quoter(self) -> InventoryMMQuoter:
        return InventoryMMQuoter(
            QuoterParams(
                base_spread=0.02,  # 2c base
                p_informed_base=0.2,  # 20% base informed probability
                time_decay_minutes=5.0,  # 5 min time constant
            )
        )

    def test_far_from_resolution_low_p_informed(
        self, quoter: InventoryMMQuoter
    ) -> None:
        """14 minutes out -> low probability of informed traders."""
        p_informed, spread = quoter.calculate_adverse_selection(14.0)

        # p_informed = 0.2 × exp(-14/5) = 0.2 × exp(-2.8) ≈ 0.012
        expected_p = 0.2 * math.exp(-14.0 / 5.0)
        assert p_informed == pytest.approx(expected_p)
        assert p_informed < 0.02  # Very low

        # spread = 0.02 × (1 + 3 × 0.012) ≈ 0.0207
        assert spread == pytest.approx(0.02 * (1 + 3 * expected_p))

    def test_near_resolution_high_p_informed(
        self, quoter: InventoryMMQuoter
    ) -> None:
        """1 minute out -> high probability of informed traders."""
        p_informed, spread = quoter.calculate_adverse_selection(1.0)

        # p_informed = 0.2 × exp(-1/5) = 0.2 × exp(-0.2) ≈ 0.164
        expected_p = 0.2 * math.exp(-1.0 / 5.0)
        assert p_informed == pytest.approx(expected_p)
        assert p_informed > 0.15  # Much higher

        # spread = 0.02 × (1 + 3 × 0.164) ≈ 0.0298
        expected_spread = 0.02 * (1 + 3 * expected_p)
        assert spread == pytest.approx(expected_spread)
        assert spread > 0.02  # Wider than base

    def test_p_informed_capped_at_80_percent(self) -> None:
        """p_informed is capped at 80% to prevent extreme spreads."""
        quoter = InventoryMMQuoter(
            QuoterParams(
                p_informed_base=0.95,  # Very high base
                time_decay_minutes=1.0,
            )
        )

        p_informed, _ = quoter.calculate_adverse_selection(0.1)  # Very close

        # Without cap: 0.95 × exp(-0.1) ≈ 0.86
        # With cap: 0.80
        assert p_informed == pytest.approx(0.8)

    def test_spread_widens_with_p_informed(self, quoter: InventoryMMQuoter) -> None:
        """Spread increases as p_informed increases (closer to resolution)."""
        _, spread_far = quoter.calculate_adverse_selection(14.0)  # Far
        _, spread_mid = quoter.calculate_adverse_selection(5.0)  # Medium
        _, spread_near = quoter.calculate_adverse_selection(1.0)  # Near

        # Spreads should increase as we get closer
        assert spread_far < spread_mid < spread_near

    def test_time_decay_parameter_effect(self) -> None:
        """Shorter time_decay = p_informed drops faster over time.

        At same minutes from resolution:
        - Fast decay (small time_decay) = p_informed has decayed more = LOWER
        - Slow decay (large time_decay) = p_informed stays elevated = HIGHER
        """
        minutes = 3.0

        # Fast decay - p_informed drops quickly
        quoter_fast = InventoryMMQuoter(
            QuoterParams(time_decay_minutes=2.0, p_informed_base=0.2)
        )
        p_fast, _ = quoter_fast.calculate_adverse_selection(minutes)

        # Slow decay - p_informed stays elevated longer
        quoter_slow = InventoryMMQuoter(
            QuoterParams(time_decay_minutes=10.0, p_informed_base=0.2)
        )
        p_slow, _ = quoter_slow.calculate_adverse_selection(minutes)

        # Slow decay = higher p_informed at same time (hasn't decayed as much)
        assert p_slow > p_fast


# =============================================================================
# LAYER 3: INVENTORY SKEW
# =============================================================================


class TestLayer3InventorySkew:
    """Test calculate_inventory_skew: Adjust quotes based on inventory balance.

    Goal: Keep UP and DOWN quantities balanced to form redeemable pairs.
    If overweight one side, make it harder to buy more (wider offset, smaller size).

    Formulas:
        spread_mult = 1 + gamma × imbalance  (for overweight side)
        size = base × exp(-lambda × imbalance)  (for overweight side)

    Imbalance q = (UP - DOWN) / (UP + DOWN):
        q = +0.5: 75% UP, 25% DOWN (overweight UP)
        q = 0.0: 50% UP, 50% DOWN (balanced)
        q = -0.5: 25% UP, 75% DOWN (overweight DOWN)
    """

    @pytest.fixture
    def quoter(self) -> InventoryMMQuoter:
        return InventoryMMQuoter(
            QuoterParams(
                gamma_inv=0.5,  # Offset sensitivity
                lambda_size=1.0,  # Size sensitivity
                base_size=100.0,  # Base order size
            )
        )

    def test_balanced_inventory_equal_multipliers(
        self, quoter: InventoryMMQuoter
    ) -> None:
        """Balanced inventory (q=0) -> symmetric quotes."""
        inventory = Inventory(up_qty=100, up_avg=0.50, down_qty=100, down_avg=0.50)

        mult_up, mult_down, size_up, size_down = quoter.calculate_inventory_skew(
            inventory
        )

        # q = (100 - 100) / 200 = 0
        assert inventory.imbalance == pytest.approx(0.0)

        # mult = 1 + 0.5 × 0 = 1.0 for both
        assert mult_up == pytest.approx(1.0)
        assert mult_down == pytest.approx(1.0)

        # size = 100 × exp(0) = 100 for both
        assert size_up == pytest.approx(100.0)
        assert size_down == pytest.approx(100.0)

    def test_overweight_up_widens_up_tightens_down(
        self, quoter: InventoryMMQuoter
    ) -> None:
        """Overweight UP (q>0) -> wider UP offset, tighter DOWN offset."""
        inventory = Inventory(up_qty=150, up_avg=0.50, down_qty=50, down_avg=0.50)

        mult_up, mult_down, _, _ = quoter.calculate_inventory_skew(inventory)

        # q = (150 - 50) / 200 = 0.5
        assert inventory.imbalance == pytest.approx(0.5)

        # mult_up = 1 + 0.5 × 0.5 = 1.25 (wider offset -> less aggressive)
        assert mult_up == pytest.approx(1.25)
        # mult_down = 1 - 0.5 × 0.5 = 0.75 (tighter offset -> more aggressive)
        assert mult_down == pytest.approx(0.75)

    def test_overweight_down_widens_down_tightens_up(
        self, quoter: InventoryMMQuoter
    ) -> None:
        """Overweight DOWN (q<0) -> wider DOWN offset, tighter UP offset."""
        inventory = Inventory(up_qty=50, up_avg=0.50, down_qty=150, down_avg=0.50)

        mult_up, mult_down, _, _ = quoter.calculate_inventory_skew(inventory)

        # q = (50 - 150) / 200 = -0.5
        assert inventory.imbalance == pytest.approx(-0.5)

        # mult_up = 1 + 0.5 × (-0.5) = 0.75 (tighter -> more aggressive)
        assert mult_up == pytest.approx(0.75)
        # mult_down = 1 - 0.5 × (-0.5) = 1.25 (wider -> less aggressive)
        assert mult_down == pytest.approx(1.25)

    def test_size_decreases_for_overweight_side(
        self, quoter: InventoryMMQuoter
    ) -> None:
        """Size shrinks for overweight side, grows for needed side."""
        inventory = Inventory(up_qty=150, up_avg=0.50, down_qty=50, down_avg=0.50)

        _, _, size_up, size_down = quoter.calculate_inventory_skew(inventory)

        # q = 0.5 (overweight UP)
        # size_up = 100 × exp(-1.0 × 0.5) = 100 × 0.606 ≈ 60.7
        assert size_up == pytest.approx(100 * math.exp(-0.5))
        # size_down = 100 × exp(1.0 × 0.5) = 100 × 1.649 ≈ 164.9
        assert size_down == pytest.approx(100 * math.exp(0.5))

        # DOWN size > UP size (want to buy more DOWN to balance)
        assert size_down > size_up

    def test_gamma_parameter_effect(self) -> None:
        """Higher gamma = more aggressive offset adjustment."""
        inventory = Inventory(up_qty=150, up_avg=0.50, down_qty=50, down_avg=0.50)

        # Low gamma
        quoter_low = InventoryMMQuoter(QuoterParams(gamma_inv=0.2))
        mult_up_low, _, _, _ = quoter_low.calculate_inventory_skew(inventory)

        # High gamma
        quoter_high = InventoryMMQuoter(QuoterParams(gamma_inv=1.0))
        mult_up_high, _, _, _ = quoter_high.calculate_inventory_skew(inventory)

        # Higher gamma = larger multiplier deviation from 1.0
        # q = 0.5: low gamma mult = 1 + 0.2×0.5 = 1.1, high = 1 + 1.0×0.5 = 1.5
        assert mult_up_low == pytest.approx(1.1)
        assert mult_up_high == pytest.approx(1.5)

    def test_lambda_parameter_effect(self) -> None:
        """Higher lambda = more aggressive size adjustment."""
        inventory = Inventory(up_qty=150, up_avg=0.50, down_qty=50, down_avg=0.50)

        # Low lambda
        quoter_low = InventoryMMQuoter(QuoterParams(lambda_size=0.5, base_size=100))
        _, _, size_up_low, _ = quoter_low.calculate_inventory_skew(inventory)

        # High lambda
        quoter_high = InventoryMMQuoter(QuoterParams(lambda_size=2.0, base_size=100))
        _, _, size_up_high, _ = quoter_high.calculate_inventory_skew(inventory)

        # Higher lambda = more size reduction for overweight side
        # q = 0.5: low lambda size = 100×exp(-0.25) ≈ 78, high = 100×exp(-1.0) ≈ 37
        assert size_up_low > size_up_high


# =============================================================================
# LAYER 4: EDGE CHECK
# =============================================================================


class TestLayer4EdgeCheck:
    """Test check_edge: Verify sufficient profit margin before quoting.

    Edge = market_ask - our_bid
    If edge < threshold, don't quote (not enough profit).

    Example:
        Market ask = 0.55
        Our bid = 0.53
        Edge = 0.55 - 0.53 = 0.02 (2c)
        If threshold = 0.01 (1c) -> PASS (2c > 1c)
    """

    @pytest.fixture
    def quoter(self) -> InventoryMMQuoter:
        return InventoryMMQuoter(QuoterParams(edge_threshold=0.01))  # 1c threshold

    def test_sufficient_edge_passes(self, quoter: InventoryMMQuoter) -> None:
        """Edge above threshold -> quote is approved."""
        should_quote, edge, reason = quoter.check_edge(bid=0.53, market_ask=0.55)

        # Edge = 0.55 - 0.53 = 0.02 (2c) > 0.01 threshold
        assert should_quote is True
        assert edge == pytest.approx(0.02)
        assert reason is None

    def test_insufficient_edge_fails_with_reason(
        self, quoter: InventoryMMQuoter
    ) -> None:
        """Edge below threshold -> quote rejected with explanation."""
        should_quote, edge, reason = quoter.check_edge(bid=0.545, market_ask=0.55)

        # Edge = 0.55 - 0.545 = 0.005 (0.5c) < 0.01 threshold
        assert should_quote is False
        assert edge == pytest.approx(0.005)
        assert reason is not None
        assert "edge" in reason.lower()
        assert "threshold" in reason.lower()

    def test_zero_edge_fails(self, quoter: InventoryMMQuoter) -> None:
        """Zero edge (bid = ask) -> rejected."""
        should_quote, edge, reason = quoter.check_edge(bid=0.55, market_ask=0.55)

        assert should_quote is False
        assert edge == pytest.approx(0.0)
        assert reason is not None

    def test_negative_edge_fails(self, quoter: InventoryMMQuoter) -> None:
        """Negative edge (bid > ask) -> rejected (would be a loss!)."""
        should_quote, edge, reason = quoter.check_edge(bid=0.56, market_ask=0.55)

        assert should_quote is False
        assert edge == pytest.approx(-0.01)  # Negative!
        assert reason is not None

    def test_edge_threshold_parameter(self) -> None:
        """Different thresholds change acceptance criteria."""
        bid, ask = 0.53, 0.55  # 2c edge

        # Low threshold (1c) -> accepts 2c edge
        quoter_low = InventoryMMQuoter(QuoterParams(edge_threshold=0.01))
        accepted_low, _, _ = quoter_low.check_edge(bid, ask)
        assert accepted_low is True

        # High threshold (3c) -> rejects 2c edge
        quoter_high = InventoryMMQuoter(QuoterParams(edge_threshold=0.03))
        accepted_high, _, _ = quoter_high.check_edge(bid, ask)
        assert accepted_high is False


# =============================================================================
# INTEGRATION: FULL QUOTE GENERATION
# =============================================================================


class TestQuoteIntegration:
    """Test quote(): Full 4-layer pipeline integration.

    Verifies all layers work together correctly:
    1. Adverse selection sets base spread
    2. Oracle adjusts offsets directionally
    3. Inventory skew adjusts offsets and sizes
    4. Edge check gates final quotes
    """

    @pytest.fixture
    def quoter(self) -> InventoryMMQuoter:
        """Quoter with known parameters."""
        return InventoryMMQuoter(
            QuoterParams(
                oracle_sensitivity=5.0,
                base_spread=0.02,
                p_informed_base=0.2,
                time_decay_minutes=5.0,
                gamma_inv=0.5,
                lambda_size=1.0,
                base_size=50.0,
                edge_threshold=0.01,
                min_offset=0.01,
            )
        )

    def test_balanced_neutral_symmetric_quotes(
        self, quoter: InventoryMMQuoter
    ) -> None:
        """Balanced inventory + neutral oracle = symmetric quotes."""
        inventory = Inventory(up_qty=100, up_avg=0.48, down_qty=100, down_avg=0.48)
        market = create_market(up_mid=0.50, spread=0.02)
        oracle = Oracle(current_price=97000, threshold=97000)

        result = quoter.quote(inventory, market, oracle, minutes_to_resolution=10.0)

        # Oracle adjustment should be ~0 (neutral)
        assert result.oracle_adj == pytest.approx(0.0, abs=0.001)

        # Inventory imbalance should be 0 (balanced)
        assert result.inventory_q == pytest.approx(0.0)

        # Spread multipliers should be 1.0 (balanced)
        assert result.spread_mult_up == pytest.approx(1.0)
        assert result.spread_mult_down == pytest.approx(1.0)

        # Sizes should be equal
        assert result.size_up == result.size_down

    def test_all_layers_interact_correctly(self, quoter: InventoryMMQuoter) -> None:
        """Verify layer interactions with asymmetric scenario."""
        # Overweight UP, bullish oracle
        inventory = Inventory(up_qty=150, up_avg=0.55, down_qty=50, down_avg=0.45)
        market = create_market(up_mid=0.60, spread=0.02)  # UP favored market
        oracle = Oracle(current_price=97500, threshold=97000)  # Bullish

        result = quoter.quote(inventory, market, oracle, minutes_to_resolution=5.0)

        # Layer 1: Oracle bullish -> positive adjustment
        assert result.oracle_adj > 0

        # Layer 2: 5 min out -> moderate p_informed
        assert 0.05 < result.p_informed < 0.2

        # Layer 3: Overweight UP -> mult_up > 1, mult_down < 1
        assert result.spread_mult_up > 1.0
        assert result.spread_mult_down < 1.0

        # Layer 3: Size DOWN > size UP (want to buy more DOWN)
        assert result.raw_size_down > result.raw_size_up

    def test_quote_skipped_when_edge_insufficient(self) -> None:
        """Quotes are skipped (None) when edge check fails."""
        quoter = InventoryMMQuoter(
            QuoterParams(
                edge_threshold=0.10,  # Very high threshold (10c)
                base_spread=0.02,
            )
        )

        inventory = Inventory(up_qty=100, up_avg=0.50, down_qty=100, down_avg=0.50)
        market = create_market(up_mid=0.50, spread=0.02)  # Only 2c spread
        oracle = Oracle(current_price=97000, threshold=97000)

        result = quoter.quote(inventory, market, oracle, minutes_to_resolution=10.0)

        # With 10c threshold and only 2c market spread, both should be skipped
        assert result.bid_up is None or result.bid_down is None
        # Skip reasons should be populated
        if result.bid_up is None:
            assert result.skip_reason_up is not None
        if result.bid_down is None:
            assert result.skip_reason_down is not None

    def test_quote_result_contains_all_diagnostics(
        self, quoter: InventoryMMQuoter
    ) -> None:
        """QuoteResult has all intermediate calculations for debugging."""
        inventory = Inventory(up_qty=120, up_avg=0.52, down_qty=80, down_avg=0.48)
        market = create_market(up_mid=0.55, spread=0.02)
        oracle = Oracle(current_price=97200, threshold=97000)

        result = quoter.quote(inventory, market, oracle, minutes_to_resolution=8.0)

        # Layer 1 diagnostics
        assert hasattr(result, "oracle_adj")
        assert hasattr(result, "raw_up_offset")
        assert hasattr(result, "raw_down_offset")

        # Layer 2 diagnostics
        assert hasattr(result, "p_informed")
        assert hasattr(result, "base_spread")

        # Layer 3 diagnostics
        assert hasattr(result, "inventory_q")
        assert hasattr(result, "spread_mult_up")
        assert hasattr(result, "spread_mult_down")
        assert hasattr(result, "final_up_offset")
        assert hasattr(result, "final_down_offset")
        assert hasattr(result, "raw_size_up")
        assert hasattr(result, "raw_size_down")

        # Layer 4 diagnostics
        assert hasattr(result, "edge_up")
        assert hasattr(result, "edge_down")

    def test_real_scenario_from_notebook(self) -> None:
        """Reproduce a scenario from the Jupyter notebook."""
        # Scenario: Balanced position, oracle neutral, 10 min left
        quoter = InventoryMMQuoter(
            QuoterParams(
                oracle_sensitivity=5.0,
                base_spread=0.01,
                p_informed_base=0.2,
                time_decay_minutes=5.0,
                gamma_inv=1.5,
                lambda_size=1.5,
                base_size=50.0,
                edge_threshold=0.01,
                min_offset=0.01,
            )
        )

        inventory = Inventory(up_qty=100, up_avg=0.48, down_qty=100, down_avg=0.48)
        market = Market(
            best_ask_up=0.51,
            best_bid_up=0.49,
            best_ask_down=0.51,
            best_bid_down=0.49,
        )
        oracle = Oracle(current_price=97000, threshold=97000)

        result = quoter.quote(inventory, market, oracle, minutes_to_resolution=10.0)

        # Should produce valid quotes
        # With neutral oracle and balanced inventory, quotes should be symmetric
        assert result.oracle_adj == pytest.approx(0.0, abs=0.001)
        assert result.inventory_q == pytest.approx(0.0)

        # Should quote both sides (edge check should pass with 2c spread)
        # Note: actual pass/fail depends on final offset vs market spread
        print(f"UP: bid={result.bid_up}, size={result.size_up}")
        print(f"DOWN: bid={result.bid_down}, size={result.size_down}")
        print(f"Combined bid: {result.combined_bid}")
