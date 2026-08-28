# MonkeyMesh CPU Miner

**Status: ACTIVE** — CLI `mesh-miner-cpu`; the all-in-one GUI also runs the CPU lane.

Algorithm: MeshHash-CPU / Evo v3 / Fusion v4 (lane A + verify lane B). Live testnet is Fusion from height 80.

Honor `pow_version` + work seed from `getblocktemplate` (`Build/31`, `Build/32`). Public mine target: `https://eu.hashmonkeys.cloud`.

Features:
- Solo mining
- Pool mining
- Auto benchmark
- HugePages support
- NUMA awareness
- Thread tuning

Platforms:
- Ubuntu
- Windows
