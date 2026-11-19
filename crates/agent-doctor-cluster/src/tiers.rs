//! Beginner-friendly model tiers and unified memory headroom rules.

use serde::{Deserialize, Serialize};

/// Leave at least 4 GiB per node for macOS / foreground apps so placing an LLM
/// never drives unified memory into swap thrashing or OOM kills.
pub const SYSTEM_MEMORY_HEADROOM_BYTES: u64 = 4 * 1024 * 1024 * 1024;

const GIB: u64 = 1024 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterModelTier {
    /// Stable tier key: `fast`, `balanced`, `large`.
    pub id: String,
    /// Underlying MLX / GGUF model identifier (hidden inside "详情" for beginners).
    pub model_id: String,
    /// Minimum usable cluster memory (bytes, after per-node headroom) recommended.
    pub min_usable_bytes: u64,
    /// Whether the current cluster has enough usable memory for this tier.
    pub fits_cluster: bool,
    /// Whether this tier is the automatic recommendation for the current cluster.
    pub recommended: bool,
    /// Download size in bytes. Zero when this row is only a size bucket.
    #[serde(default)]
    pub size_bytes: u64,
    /// The model files are already on this computer.
    #[serde(default)]
    pub on_disk: bool,
}

pub fn model_tiers_for_usable_memory(usable_bytes: u64) -> Vec<ClusterModelTier> {
    let specs = [
        ("fast", "mlx-community/Qwen2.5-7B-Instruct-4bit", 6 * GIB),
        ("balanced", "mlx-community/Qwen3-30B-A3B-4bit", 22 * GIB),
        ("large", "mlx-community/Qwen2.5-72B-Instruct-4bit", 48 * GIB),
    ];

    // Pick the largest tier that fits; if usable_bytes is 0 (not yet reported),
    // recommend `fast` as a safe starting point.
    let recommended_id = specs
        .iter()
        .rev()
        .find(|(_, _, min_bytes)| usable_bytes >= *min_bytes)
        .map(|(id, _, _)| *id)
        .unwrap_or("fast");

    specs
        .into_iter()
        .map(|(id, model_id, min_usable_bytes)| ClusterModelTier {
            id: id.to_string(),
            model_id: model_id.to_string(),
            min_usable_bytes,
            fits_cluster: usable_bytes == 0 || usable_bytes >= min_usable_bytes,
            recommended: id == recommended_id,
            size_bytes: 0,
            on_disk: false,
        })
        .collect()
}

/// Subtract `SYSTEM_MEMORY_HEADROOM_BYTES` per online node from reported available RAM.
pub fn usable_cluster_memory_bytes(avail_per_node: &[u64]) -> u64 {
    avail_per_node
        .iter()
        .map(|avail| avail.saturating_sub(SYSTEM_MEMORY_HEADROOM_BYTES))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_16gb_mac_recommends_fast_and_two_32gb_macs_unlock_large() {
        let one_mac = usable_cluster_memory_bytes(&[14 * GIB]);
        let tiers_one = model_tiers_for_usable_memory(one_mac);
        assert!(
            tiers_one
                .iter()
                .find(|t| t.id == "fast")
                .unwrap()
                .recommended
        );
        assert!(
            !tiers_one
                .iter()
                .find(|t| t.id == "large")
                .unwrap()
                .fits_cluster
        );

        let two_macs = usable_cluster_memory_bytes(&[28 * GIB, 28 * GIB]);
        assert_eq!(two_macs, 48 * GIB);
        let tiers_two = model_tiers_for_usable_memory(two_macs);
        assert!(
            tiers_two
                .iter()
                .find(|t| t.id == "large")
                .unwrap()
                .recommended
        );
        assert!(
            tiers_two
                .iter()
                .find(|t| t.id == "large")
                .unwrap()
                .fits_cluster
        );
    }
}
