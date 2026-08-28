MonkeyMesh Orchestrator + AI worker (Windows)

Seed node embeds the AI job board. On your PC you usually only need the worker:

  Start-GpuWorker.bat
  (default orch/node = http://seednode.hashmonkeys.cloud:18080)

Or run a local stack:
1. Start a Node (Releases\Windows\Node)
2. Start-Orchestrator.bat (optional marketplace UI)
3. Start-GpuWorker.bat with MESH_ORCH=http://127.0.0.1:18080

Prefer the dedicated pack: Releases\Windows\AiWorker
