import { invoke } from "@tauri-apps/api/core";

const state = {
  rpc: localStorage.getItem("mesh.rpc") || "",
  address: "",
  busy: false,
};

const app = document.getElementById("app");

app.innerHTML = `
  <section class="hero">
    <div class="hero-bg" aria-hidden="true"></div>
    <div class="hero-copy">
      <div class="brand-row">
        <img class="brand-mark" src="/branding/mascot.png" alt="MonkeyMesh mascot" />
        <h1 class="brand">Monkey<span>Mesh</span></h1>
      </div>
      <p class="lede">AI-powered MESH wallet — send, receive, mine, and watch your node from one place.</p>
      <div class="cta-row">
        <button class="btn primary" id="btn-refresh">Refresh</button>
        <button class="btn" id="btn-mine">Mine 1 block</button>
        <button class="btn" id="btn-explorer">Open explorer</button>
      </div>
    </div>
  </section>

  <main class="workspace">
    <section class="panel">
      <h2>Balance</h2>
      <p class="balance" id="balance">—</p>
      <p class="muted mono" id="addr">connecting…</p>
    </section>

    <div class="grid-2">
      <section class="panel">
        <h2>Send MESH</h2>
        <label for="to">To address</label>
        <input id="to" placeholder="mesh01…" autocomplete="off" />
        <label for="amount">Amount</label>
        <input id="amount" placeholder="1.5" autocomplete="off" />
        <label for="memo">Memo</label>
        <input id="memo" placeholder="optional" autocomplete="off" />
        <button class="btn primary" id="btn-send">Send</button>
        <p class="status" id="send-status"></p>
      </section>

      <section class="panel">
        <h2>Receive</h2>
        <div class="receive-row">
          <img src="/branding/coin_mark.png" alt="MESH coin mark" />
          <div>
            <p class="muted">Share this address to receive MESH</p>
            <p class="mono" id="recv-addr">—</p>
            <button class="btn" id="btn-copy" style="margin-top:0.75rem">Copy address</button>
          </div>
        </div>
      </section>
    </div>

    <section class="panel">
      <h2>Node</h2>
      <div class="stat-row" id="node-stats"></div>
      <label for="rpc" style="margin-top:1rem">RPC URL</label>
      <input id="rpc" value="" />
      <button class="btn" id="btn-save-rpc">Save RPC</button>
      <p class="status" id="node-status"></p>
    </section>
  </main>

  <footer class="note">Launchers/Wallet · connects to Launchers/Node RPC</footer>
`;

const $ = (id) => document.getElementById(id);

async function api(cmd, args = {}) {
  const payload = { ...args };
  if (state.rpc) payload.rpcUrl = state.rpc;
  return invoke(cmd, payload);
}

function setBusy(v) {
  state.busy = v;
  ["btn-refresh", "btn-mine", "btn-send"].forEach((id) => {
    $(id).disabled = v;
  });
}

async function boot() {
  try {
    const settings = await invoke("get_settings");
    if (!state.rpc && settings?.rpc) {
      state.rpc = settings.rpc;
    }
  } catch (_) {
    if (!state.rpc) state.rpc = "http://127.0.0.1:18080";
  }
  $("rpc").value = state.rpc;
  await refresh();
}

async function refresh() {
  setBusy(true);
  $("node-status").textContent = "";
  $("node-status").className = "status";
  try {
    const info = await api("get_node_info");
    const wallet = await api("get_wallet");
    state.address = wallet.address;
    $("addr").textContent = wallet.address;
    $("recv-addr").textContent = wallet.address;
    $("balance").textContent = wallet.balance;

    $("node-stats").innerHTML = [
      ["Height", info.height],
      ["Next diff", info.next_difficulty],
      ["Mempool", info.mempool],
      ["Finality", info.finality_active ? "#" + info.finalized_height : "off"],
      ["Peer", short(info.peer_id || "local")],
    ]
      .map(([k, v]) => `<div class="stat"><b>${v}</b><span>${k}</span></div>`)
      .join("");

    $("node-status").textContent = "node online";
    $("node-status").className = "status ok";
  } catch (e) {
    $("balance").textContent = "—";
    $("addr").textContent = String(e);
    $("node-status").textContent = String(e);
    $("node-status").className = "status err";
  } finally {
    setBusy(false);
  }
}

function short(s) {
  if (!s || s.length < 18) return s || "—";
  return s.slice(0, 10) + "…" + s.slice(-6);
}

$("btn-refresh").onclick = () => refresh();
$("btn-save-rpc").onclick = () => {
  state.rpc = $("rpc").value.trim().replace(/\/$/, "");
  localStorage.setItem("mesh.rpc", state.rpc);
  refresh();
};
$("btn-copy").onclick = async () => {
  if (!state.address) return;
  await navigator.clipboard.writeText(state.address);
  $("send-status").textContent = "address copied";
  $("send-status").className = "status ok";
};
$("btn-explorer").onclick = () => {
  window.open(state.rpc + "/", "_blank");
};
$("btn-mine").onclick = async () => {
  setBusy(true);
  $("send-status").textContent = "mining…";
  $("send-status").className = "status";
  try {
    const res = await api("mine_blocks", { blocks: 1 });
    $("send-status").textContent = `mined height ${res.height}`;
    $("send-status").className = "status ok";
    await refresh();
  } catch (e) {
    $("send-status").textContent = String(e);
    $("send-status").className = "status err";
  } finally {
    setBusy(false);
  }
};
$("btn-send").onclick = async () => {
  setBusy(true);
  $("send-status").textContent = "sending…";
  $("send-status").className = "status";
  try {
    const res = await api("send_mesh", {
      to: $("to").value.trim(),
      amount: $("amount").value.trim(),
      memo: $("memo").value.trim(),
    });
    $("send-status").textContent = `submitted ${res.txid}`;
    $("send-status").className = "status ok";
    $("to").value = "";
    $("amount").value = "";
    $("memo").value = "";
    await refresh();
  } catch (e) {
    $("send-status").textContent = String(e);
    $("send-status").className = "status err";
  } finally {
    setBusy(false);
  }
};

boot();
