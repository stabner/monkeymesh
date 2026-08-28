MonkeyMesh CPU Miner (portable pack)
===================================

Everything you need is in THIS folder. Copy the whole folder anywhere.

Contents
  MonkeyMesh-CpuMiner.exe
  vcruntime140.dll / vcruntime140_1.dll / msvcp140.dll / concrt140.dll
  config.json
  Start-CpuMiner.bat

Setup
  1. Start your MonkeyMesh Node.
  2. Edit config.json -> set "address" to your mesh01... payout address.
  3. Double-click Start-CpuMiner.bat

blocks: 0 means mine until Ctrl+C.

From height 29,000 the official CPU miner refuses sequential Fusion (v5).
Use Releases\Windows\Miner or GpuMiner (GPU required) before that height.
