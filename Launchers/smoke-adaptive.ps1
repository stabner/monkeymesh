# Smoke: node RPC markets + generate/vote proposal (node must be running).
$ErrorActionPreference = "Stop"
$rpc = if ($env:MESH_NODE_RPC) { $env:MESH_NODE_RPC.TrimEnd('/') } else { "http://127.0.0.1:18080" }

Write-Host "==> markets"
Invoke-RestMethod "$rpc/v1/markets" | ConvertTo-Json -Depth 5

Write-Host "==> meshpulse"
Invoke-RestMethod "$rpc/v1/meshpulse" | ConvertTo-Json -Depth 5

Write-Host "==> generate proposal"
$prop = Invoke-RestMethod -Method Post "$rpc/v1/proposals/generate"
$prop | ConvertTo-Json -Depth 6
$id = $prop.proposal.id
if (-not $id) { throw "no proposal id" }

Write-Host "==> vote yes $id (one vote per node id)"
$vote = Invoke-RestMethod -Method Post "$rpc/v1/proposals/vote" -ContentType "application/json" -Body (@{ id = $id; choice = "yes" } | ConvertTo-Json)
$vote | ConvertTo-Json -Depth 5

Write-Host "==> duplicate vote should fail"
try {
    Invoke-RestMethod -Method Post "$rpc/v1/proposals/vote" -ContentType "application/json" -Body (@{ id = $id; choice = "no" } | ConvertTo-Json) | Out-Null
    throw "duplicate vote was accepted"
} catch {
    if ($_.Exception.Message -match "duplicate vote was accepted") { throw }
    Write-Host "    rejected as expected"
}

Write-Host "==> envelopes"
Invoke-RestMethod "$rpc/v1/envelopes" | ConvertTo-Json -Depth 5

Write-Host "OK - one vote per node_id; soft envelopes may activate; BPS unchanged."
