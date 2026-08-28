MonkeyMesh Ubuntu / Linux releases

Layout:
  Node/           mesh-node + start-node.sh + config.json
  Orchestrator/   mesh-orchestrator + mesh-gpu-worker + start scripts
  CpuMiner/       mesh-miner-cpu + start-cpu-miner.sh

Build on Linux (or pull from MESH_NAS_HOST after mesh-testnet.sh build):
  ~/src/MonkeyMesh/Launchers/testnet/mesh-testnet.sh build

GPU GUI miners are Windows-focused; Linux uses the CLI CPU miner + GPU research worker.
