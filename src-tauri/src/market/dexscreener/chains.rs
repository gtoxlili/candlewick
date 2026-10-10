//! The chains pools live on, by the id DEX Screener gives them: what
//! GeckoTerminal calls the same chain (its ids differ for a few big ones),
//! and the name to show. A chain not listed here keeps its id on both
//! services and is named by capitalizing it, so new chains work without
//! an update, just plainer.
//!
//! Built from GeckoTerminal's `/networks` (226 entries on 2026-10-11) and
//! the slugs in DEX Screener's pair URLs (`dexscreener.com/<chain>/<pool>`).

struct Chain {
    screener: &'static str,
    gecko: &'static str,
    name: &'static str,
}

const fn chain(screener: &'static str, gecko: &'static str, name: &'static str) -> Chain {
    Chain { screener, gecko, name }
}

const CHAINS: &[Chain] = &[
    chain("solana", "solana", "Solana"),
    chain("ethereum", "eth", "Ethereum"),
    chain("bsc", "bsc", "BNB Chain"),
    chain("base", "base", "Base"),
    chain("arbitrum", "arbitrum", "Arbitrum"),
    chain("polygon", "polygon_pos", "Polygon"),
    chain("avalanche", "avax", "Avalanche"),
    chain("optimism", "optimism", "Optimism"),
    chain("sui", "sui-network", "Sui"),
    chain("ton", "ton", "TON"),
    chain("tron", "tron", "Tron"),
    chain("hyperevm", "hyperevm", "HyperEVM"),
    chain("hyperliquid", "hyperliquid", "Hyperliquid"),
    chain("sonic", "sonic", "Sonic"),
    chain("abstract", "abstract", "Abstract"),
    chain("berachain", "berachain", "Berachain"),
    chain("unichain", "unichain", "Unichain"),
    chain("monad", "monad", "Monad"),
    chain("megaeth", "megaeth", "MegaETH"),
    chain("pulsechain", "pulsechain", "PulseChain"),
    chain("blast", "blast", "Blast"),
    chain("linea", "linea", "Linea"),
    chain("scroll", "scroll", "Scroll"),
    chain("mantle", "mantle", "Mantle"),
    chain("zksync", "zksync", "zkSync"),
    chain("polygonzkevm", "polygon-zkevm", "Polygon zkEVM"),
    chain("opbnb", "opbnb", "opBNB"),
    chain("cronos", "cro", "Cronos"),
    chain("fantom", "ftm", "Fantom"),
    chain("sei", "sei-evm", "Sei"),
    chain("aptos", "aptos", "Aptos"),
    chain("near", "near", "NEAR"),
    chain("starknet", "starknet-alpha", "Starknet"),
    chain("cardano", "cardano", "Cardano"),
    chain("icp", "icp", "Internet Computer"),
    chain("xrpl", "xrpl", "XRPL"),
    chain("stellar", "stellar", "Stellar"),
    chain("injective", "injective", "Injective"),
    chain("worldchain", "world-chain", "World Chain"),
    chain("apechain", "apechain", "ApeChain"),
    chain("soneium", "soneium", "Soneium"),
    chain("ink", "ink", "Ink"),
    chain("plasma", "plasma", "Plasma"),
    chain("story", "story", "Story"),
    chain("celo", "celo", "Celo"),
    chain("manta", "manta-pacific", "Manta"),
    chain("mode", "mode", "Mode"),
    chain("zora", "zora-network", "Zora"),
    chain("fraxtal", "fraxtal", "Fraxtal"),
    chain("taiko", "taiko", "Taiko"),
    chain("hedera", "hedera-hashgraph", "Hedera"),
    chain("flow", "flow-evm", "Flow"),
    chain("plume", "plume-network", "Plume"),
    chain("katana", "katana", "Katana"),
    chain("zircuit", "zircuit", "Zircuit"),
    chain("lisk", "lisk", "Lisk"),
    chain("moonbeam", "glmr", "Moonbeam"),
    chain("moonriver", "movr", "Moonriver"),
    chain("harmony", "one", "Harmony"),
    chain("metis", "metis", "Metis"),
    chain("gnosis", "xdai", "Gnosis"),
    chain("kava", "kava", "Kava"),
    chain("telos", "tlos", "Telos"),
    chain("iotex", "iotx", "IoTeX"),
    chain("astar", "astr", "Astar"),
    chain("conflux", "cfx", "Conflux"),
    chain("core", "core", "Core"),
    chain("shibarium", "shibarium", "Shibarium"),
    chain("merlinchain", "merlin-chain", "Merlin"),
    chain("bitlayer", "bitlayer", "Bitlayer"),
    chain("xlayer", "x-layer", "X Layer"),
    chain("robinhood", "robinhood", "Robinhood Chain"),
];

fn find(screener: &str) -> Option<&'static Chain> {
    CHAINS.iter().find(|c| c.screener == screener)
}

/// The chain's id on GeckoTerminal.
pub fn gecko_network(screener: &str) -> &str {
    find(screener).map_or(screener, |c| c.gecko)
}

/// The chain's name: `Solana`, `BNB Chain`.
pub fn name(screener: &str) -> String {
    find(screener).map_or_else(|| capitalized(screener), |c| c.name.to_owned())
}

/// A DEX's name from its DEX Screener id: `raydium` → `Raydium`.
pub fn dex_name(id: &str) -> String {
    const KNOWN: &[(&str, &str)] = &[
        ("raydium", "Raydium"),
        ("raydium-clmm", "Raydium CLMM"),
        ("orca", "Orca"),
        ("meteora", "Meteora"),
        ("meteora-dlmm", "Meteora DLMM"),
        ("pumpswap", "PumpSwap"),
        ("pumpfun", "Pump.fun"),
        ("launchlab", "LaunchLab"),
        ("fluxbeam", "FluxBeam"),
        ("lifinity", "Lifinity"),
        ("uniswap", "Uniswap"),
        ("sushiswap", "SushiSwap"),
        ("pancakeswap", "PancakeSwap"),
        ("aerodrome", "Aerodrome"),
        ("velodrome", "Velodrome"),
        ("curve", "Curve"),
        ("balancer", "Balancer"),
        ("camelot", "Camelot"),
        ("quickswap", "QuickSwap"),
        ("traderjoe", "Trader Joe"),
        ("fluid", "Fluid"),
        ("maverick", "Maverick"),
        ("cetus", "Cetus"),
        ("bluefin", "Bluefin"),
        ("stonfi", "STON.fi"),
        ("dedust", "DeDust"),
        ("sunswap", "SunSwap"),
        ("hyperswap", "HyperSwap"),
        ("shadow-exchange", "Shadow"),
        ("thruster", "Thruster"),
        ("baseswap", "BaseSwap"),
        ("kodiak", "Kodiak"),
        ("pulsex", "PulseX"),
    ];
    KNOWN
        .iter()
        .find(|(known, _)| *known == id)
        .map_or_else(|| capitalized(id), |(_, name)| (*name).to_owned())
}

/// `pulsechain` → `Pulsechain`.
fn capitalized(id: &str) -> String {
    let mut chars = id.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_chains_map_and_unknown_ones_pass_through() {
        assert_eq!(gecko_network("ethereum"), "eth");
        assert_eq!(gecko_network("polygon"), "polygon_pos");
        assert_eq!(gecko_network("solana"), "solana");
        assert_eq!(gecko_network("somenewchain"), "somenewchain");
        assert_eq!(name("bsc"), "BNB Chain");
        assert_eq!(name("somenewchain"), "Somenewchain");
        assert_eq!(dex_name("raydium"), "Raydium");
        assert_eq!(dex_name("someswap"), "Someswap");
    }

    #[test]
    fn ids_are_unique() {
        for (i, a) in CHAINS.iter().enumerate() {
            assert!(!CHAINS[..i].iter().any(|b| b.screener == a.screener), "{} twice", a.screener);
        }
    }
}
