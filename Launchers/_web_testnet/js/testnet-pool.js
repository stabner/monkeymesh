(function () {
  "use strict";

  const API_BASE = (window.MH_TESTNET_API_BASE || "/testnet-api").replace(/\/$/, "");
  const RPC_PUBLIC =
    window.MH_MESH_RPC || "http://seednode.hashmonkeys.cloud:18080";
  const POOL_PUBLIC =
    window.MH_MESH_POOL || "https://eu.hashmonkeys.cloud";
  const MATURITY = 20;
  const STORE_KEY = "mh.mesh.minerAddress";

  function esc(s) {
    return String(s ?? "")
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/"/g, "&quot;");
  }

  function shortAddr(a) {
    var s = String(a || "");
    if (s.length <= 18) return s;
    return s.slice(0, 10) + "…" + s.slice(-8);
  }

  function parseMesh(s) {
    var n = parseFloat(String(s || "").replace(/[^\d.]/g, ""));
    return isFinite(n) ? n : 0;
  }

  function fmtMesh(n) {
    if (!isFinite(n)) return "—";
    return n.toFixed(2).replace(/\.00$/, "") + " MESH";
  }

  async function api(path) {
    var r = await fetch(API_BASE + path, { cache: "no-store" });
    if (!r.ok) throw new Error("HTTP " + r.status);
    return r.json();
  }

  async function fetchPoolStats() {
    var urls = [API_BASE + "/v1/poolstats", POOL_PUBLIC.replace(/\/$/, "") + "/v1/poolstats"];
    for (var i = 0; i < urls.length; i++) {
      try {
        var r = await fetch(urls[i], { cache: "no-store" });
        if (r.ok) return r.json();
      } catch (e) {
        /* try next */
      }
    }
    return null;
  }

  function setStatus(msg, ok) {
    var el = document.getElementById("tpStatus");
    if (!el) return;
    el.textContent = msg;
    el.className = ok === false ? "tp-err" : "tp-muted";
  }

  function stat(label, value, hint) {
    return (
      '<div class="tp-stat"><div class="label">' +
      esc(label) +
      '</div><div class="value">' +
      value +
      "</div>" +
      (hint ? '<div class="tp-hint">' + esc(hint) + "</div>" : "") +
      "</div>"
    );
  }

  function readAddress() {
    var input = document.getElementById("tpAddr");
    var q = new URLSearchParams(location.search).get("address") || "";
    var fromStore = "";
    try {
      fromStore = localStorage.getItem(STORE_KEY) || "";
    } catch (e) {
      fromStore = "";
    }
    var addr = (input && input.value.trim()) || q.trim() || fromStore.trim();
    if (input && !input.value && addr) input.value = addr;
    return addr;
  }

  function saveAddress(addr) {
    try {
      if (addr) localStorage.setItem(STORE_KEY, addr);
    } catch (e) {
      /* ignore */
    }
  }

  function maturityOf(height, tip) {
    var h = Number(height || 0);
    var t = Number(tip || 0);
    var conf = t >= h ? t - h + 1 : 0;
    var mature = t + 1 >= h + MATURITY;
    return {
      confirmations: conf,
      remain: Math.max(0, MATURITY - conf),
      mature: mature,
    };
  }

  function maturityCell(m) {
    var conf = Math.min(Number(m.confirmations || 0), MATURITY);
    var pct = Math.round((conf / MATURITY) * 100);
    if (m.mature) {
      return (
        '<div class="tp-mat"><span class="tp-ok">spendable</span>' +
        '<div class="tp-bar is-ok" title="20/20 confirms"><i style="width:100%"></i></div></div>'
      );
    }
    return (
      '<div class="tp-mat"><span>' +
      conf +
      "/" +
      MATURITY +
      " · maturing</span>" +
      '<div class="tp-bar" title="' +
      conf +
      " of " +
      MATURITY +
      ' confirmations"><i style="width:' +
      pct +
      '%"></i></div></div>'
    );
  }

  function minerBase(s) {
    return String(s || "").split(".")[0];
  }

  function minerWorker(s) {
    var parts = String(s || "").split(".");
    return parts.length > 1 ? parts.slice(1).join(".") : "";
  }

  function groupMinerBlocks(txs, miner, tip) {
    var byH = {};
    (txs || []).forEach(function (tx) {
      if (tx.height == null || !String(tx.memo || "").startsWith("pomc:")) return;
      var h = Number(tx.height);
      var seal = 0;
      var gpu = 0;
      var node = 0;
      var mine = 0;
      (tx.outputs || []).forEach(function (o, i) {
        if (o.address !== miner) return;
        var amt = parseMesh(o.amount);
        mine += amt;
        var lane = String(o.lane || "");
        if (lane === "cpu_find" || (i === 0 && !lane)) seal += amt;
        else if (lane === "node_work" || String(o.title || "").toLowerCase().indexOf("node") >= 0)
          node += amt;
        else gpu += amt;
      });
      if (mine <= 0) return;
      if (!byH[h]) byH[h] = { height: h, seal: 0, gpu: 0, node: 0, total: 0 };
      byH[h].seal += seal;
      byH[h].gpu += gpu;
      byH[h].node += node;
      byH[h].total += mine;
    });
    return Object.keys(byH)
      .map(Number)
      .sort(function (a, b) {
        return b - a;
      })
      .map(function (h) {
        var row = byH[h];
        var m = maturityOf(h, tip);
        row.remain = m.remain;
        row.mature = m.mature;
        row.confirmations = m.confirmations;
        return row;
      });
  }

  function groupRewardHits(rewards, tip) {
    var byH = {};
    ((rewards && rewards.recent) || []).forEach(function (hit) {
      var h = Number(hit.height);
      if (!byH[h]) byH[h] = { height: h, seal: 0, gpu: 0, node: 0, total: 0 };
      var amt = parseMesh(hit.amount);
      var lane = String(hit.lane || "");
      if (lane === "cpu_find") byH[h].seal += amt;
      else if (lane === "node_work") byH[h].node += amt;
      else byH[h].gpu += amt;
      byH[h].total += amt;
      var m = maturityOf(h, tip);
      byH[h].confirmations =
        hit.confirmations != null ? Number(hit.confirmations) : m.confirmations;
      byH[h].mature = hit.mature != null ? !!hit.mature : m.mature;
      byH[h].remain = Math.max(0, MATURITY - byH[h].confirmations);
    });
    return Object.keys(byH)
      .map(Number)
      .sort(function (a, b) {
        return b - a;
      })
      .map(function (h) {
        return byH[h];
      });
  }

  async function refresh() {
    try {
      var miner = readAddress();
      var reqs = [
        api("/v1/getnodeinfo"),
        api("/v1/markets"),
        api("/v1/meshpulse").catch(function () {
          return {};
        }),
        api("/v1/miningstatus").catch(function () {
          return { active_miners: [] };
        }),
        fetchPoolStats(),
        api("/v1/exam/status").catch(function () {
          return {};
        }),
      ];
      if (miner && miner.indexOf("mesh") === 0) {
        reqs.push(api("/v1/getbalance?address=" + encodeURIComponent(miner)).catch(function () {
          return null;
        }));
        reqs.push(api("/v1/listtransactions?address=" + encodeURIComponent(miner)).catch(function () {
          return [];
        }));
        reqs.push(api("/v1/getrewards?address=" + encodeURIComponent(miner)).catch(function () {
          return null;
        }));
      } else {
        reqs.push(Promise.resolve(null));
        reqs.push(Promise.resolve([]));
        reqs.push(Promise.resolve(null));
      }

      var pair = await Promise.all(reqs);
      var info = pair[0];
      var markets = pair[1];
      var pulse = pair[2];
      var mining = pair[3];
      var pool = pair[4];
      var exam = pair[5];
      var bal = pair[6];
      var txs = pair[7] || [];
      var rewards = pair[8];

      var mkt = (pulse && pulse.markets) || {};
      var activeMiners = ((mining && mining.active_miners) || []).filter(function (m) {
        return m.mining;
      }).length;
      var poolMiners = pool && pool.connected_miners != null ? Number(pool.connected_miners) : null;
      var tip = Number(info.height || 0);
      var blocks = groupRewardHits(rewards, tip);
      if (!blocks.length) blocks = groupMinerBlocks(txs, miner, tip);
      var found = blocks.length;
      var lastTotal = found ? blocks[0].total : 0;
      var immatureCount = blocks.filter(function (b) {
        return !b.mature;
      }).length;

      var stats = document.getElementById("tpStats");
      if (stats) {
        stats.innerHTML =
          stat("Chain height", esc(info.height ?? "—"), "5s target · Fusion") +
          stat(
            "Finality",
            info.finality_active
              ? "#" + (info.finalized_height || 0)
              : "off (lab)",
            info.finality_active ? "bonded attest window" : "20 confirms spendable"
          ) +
          stat("Difficulty", esc(info.next_difficulty ?? "—"), "consensus") +
          stat(
            "Your last block",
            found ? fmtMesh(lastTotal) : "—",
            found ? "split by work type" : "paste your mesh01… address"
          ) +
          stat("Miners online", esc(poolMiners != null ? poolMiners : activeMiners), "HTTPS pool");
      }

      var you = document.getElementById("tpYou");
      if (you) {
        if (!miner) {
          you.innerHTML = '<p class="tp-muted">Paste the same <code>mesh01…</code> the miner uses (HD Address 1 if that is what you mine as).</p>';
        } else if (!bal) {
          you.innerHTML = '<p class="tp-err">Could not load balance for ' + esc(shortAddr(miner)) + "</p>";
        } else {
          you.innerHTML =
            '<div class="tp-row"><span class="tp-muted">Address</span><span class="tp-mono" title="' +
            esc(miner) +
            '">' +
            esc(shortAddr(miner)) +
            "</span></div>" +
            '<div class="tp-row"><span class="tp-muted">Spendable</span><span>' +
            esc(bal.spendable || "0") +
            "</span></div>" +
            '<div class="tp-row"><span class="tp-muted">Immature</span><span>' +
            esc(bal.immature || "0") +
            " · waits " +
            MATURITY +
            " confirms</span></div>" +
            '<div class="tp-row"><span class="tp-muted">Total</span><span><strong>' +
            esc(bal.balance || "0") +
            "</strong></span></div>" +
            '<div class="tp-row"><span class="tp-muted">Blocks found</span><span>' +
            found +
            "</span></div>" +
            (rewards && rewards.by_lane
              ? rewards.by_lane
                  .map(function (l) {
                    return (
                      '<div class="tp-row"><span class="tp-muted">' +
                      esc(l.title) +
                      "</span><span title=\"" +
                      esc(l.paid_for || "") +
                      '">' +
                      esc(l.amount) +
                      " · " +
                      esc(l.count) +
                      "</span></div>"
                    );
                  })
                  .join("")
              : "") +
            '<p class="tp-muted" style="margin-top:.65rem">Each accepted block coins <strong>45% Fusion seal + 45% GPU work</strong> to this address. Coinbase is immature for <strong>20 confirms</strong> (~100 s). Watch the bar on <a href="blocks.html#scheme=DFPPS%2B&amp;coin=MonkeyMesh">Blocks</a>' +
            (immatureCount ? " · " + immatureCount + " still maturing" : "") +
            ".</p>";
        }
      }

      var tiers = document.getElementById("tpTiers");
      if (tiers) {
        var split = markets.fair_split ? "45 / 45 / 10" : "legacy unit share";
        tiers.innerHTML =
          '<div class="tp-row"><span class="tp-muted">Split</span><span>' +
          esc(split) +
          "</span></div>" +
          '<div class="tp-row"><span class="tp-muted">CPU / finder</span><span>' +
          esc(markets.cpu_market || "—") +
          "</span></div>" +
          '<div class="tp-row"><span class="tp-muted">GPU / exam + Fusion</span><span>' +
          esc(markets.gpu_market || "—") +
          "</span></div>" +
          '<div class="tp-row"><span class="tp-muted">Nodes</span><span>' +
          esc(markets.node_market || "—") +
          "</span></div>" +
          '<p class="tp-muted" style="margin-top:.65rem">Finder takes Fusion seal 45% + GPU work 45%. Nodes take 10% only when they attest useful work. Research cannot move these BPS.</p>';
      }

      var examEl = document.getElementById("tpExam");
      if (examEl) {
        var recent = (exam && exam.recent) || [];
        if (!recent.length) {
          examEl.innerHTML = '<p class="tp-muted">No rematched exams yet this tip. Miner GUI shows Exam when a template sidecar matches.</p>';
        } else {
          examEl.innerHTML = recent
            .slice(0, 6)
            .map(function (e) {
              return (
                '<div class="tp-row"><span>' +
                esc(e.title || e.scenario || "exam") +
                '</span><span class="tp-ok">MATCH · ' +
                esc(String(e.latency_ms || 0)) +
                " ms</span></div>"
              );
            })
            .join("");
        }
      }

      var payEl = document.getElementById("tpPayout");
      if (payEl) {
        var pay = pool && (pool.payout_address || "").trim();
        if (pay && pay !== "miner") {
          payEl.innerHTML =
            '<span class="tp-mono" title="' +
            esc(pay) +
            '">' +
            esc(shortAddr(pay)) +
            '</span> <span class="tp-err">override — every find pays this wallet, not the miner</span>';
        } else {
          payEl.textContent = "the miner’s address (45 Fusion seal + 45 GPU). Spend after 20 confirms.";
        }
      }
      var pm = document.getElementById("tpPoolMiners");
      if (pm) {
        pm.textContent =
          poolMiners != null
            ? poolMiners + (pool && pool.block_height ? " · template #" + pool.block_height : "")
            : "—";
      }

      var cmd = document.getElementById("tpMinerCmd");
      if (cmd) {
        cmd.textContent =
          "Pool — Miner Mine target / config.json:\n" +
          '"rpc": "' +
          POOL_PUBLIC +
          '"\n' +
          '"address": "mesh01…"\n\n' +
          "Wallet / solo / AI — rpc:\n" +
          RPC_PUBLIC +
          "\n(+ failover http://seednode.hashmonkeys.cloud:18083)\n\n" +
          "MonkeyMesh-Node.exe → peer:\n" +
          "seednode.hashmonkeys.cloud:39001";
      }

      var sp = document.getElementById("tpRpcPublic");
      if (sp) sp.textContent = RPC_PUBLIC;

      var findings = document.getElementById("tpFindings");
      if (findings) findings.textContent = String(mkt.research_eval_receipts ?? 0);

      setStatus("Seed online · last update " + new Date().toLocaleTimeString(), true);
      var badge = document.getElementById("tpLiveBadge");
      if (badge) badge.textContent = (poolMiners || activeMiners) > 0 ? "miners" : "live";
    } catch (err) {
      setStatus("Cannot reach seed via /testnet-api — " + err.message, false);
    }
  }

  function bind() {
    var form = document.getElementById("tpAddrForm");
    var input = document.getElementById("tpAddr");
    if (form) {
      form.addEventListener("submit", function (ev) {
        ev.preventDefault();
        var addr = (input && input.value.trim()) || "";
        saveAddress(addr);
        var u = new URL(location.href);
        if (addr) u.searchParams.set("address", addr);
        else u.searchParams.delete("address");
        history.replaceState({}, "", u);
        refresh();
      });
    }
    readAddress();
    refresh();
    setInterval(refresh, 8000);
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", bind);
  } else {
    bind();
  }
})();
