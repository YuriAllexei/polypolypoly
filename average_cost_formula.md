# Average Cost Formula for Yes/No Positions

## Individual Position Average Cost

For a single position $k \in \{\text{Yes}, \text{No}\}$ with fills over time:

$$\bar{C}^k = \frac{\sum_{i=1}^{n_k} q_i^k \cdot p_i^k}{\sum_{i=1}^{n_k} q_i^k} = \frac{\sum_{i=1}^{n_k} q_i^k \cdot p_i^k}{Q^k}$$

Where:
- $n_k$ = number of fills for position $k$
- $q_i^k$ = quantity of shares in fill $i$
- $p_i^k$ = price per share in fill $i$
- $Q^k = \sum_{i=1}^{n_k} q_i^k$ = total quantity held

## Combined Average Cost (Weighted)

If you want the weighted average across both positions:

$$\bar{C}^{\text{total}} = \frac{\sum_{i=1}^{n_{\text{Yes}}} q_i^{\text{Yes}} \cdot p_i^{\text{Yes}} + \sum_{j=1}^{n_{\text{No}}} q_j^{\text{No}} \cdot p_j^{\text{No}}}{Q^{\text{Yes}} + Q^{\text{No}}}$$

## Arbitrage Cost Basis (Sum of Average Costs)

For arbitrage analysis where 1 Yes + 1 No = $1 payout:

$$C^{\text{arb}} = \bar{C}^{\text{Yes}} + \bar{C}^{\text{No}}$$

**Profit condition:** $C^{\text{arb}} < 1$ implies locked-in profit of $(1 - C^{\text{arb}})$ per share pair.

## Time-Series Notation

Using discrete time $t \in \{1, ..., T\}$:

$$\bar{C}^k = \frac{\sum_{t=1}^{T} q_t^k \cdot p_t^k \cdot \mathbb{1}_{[q_t^k > 0]}}{\sum_{t=1}^{T} q_t^k}$$

Where $\mathbb{1}_{[q_t^k > 0]}$ is the indicator function for fills occurring at time $t$.
