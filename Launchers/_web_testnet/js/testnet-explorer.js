(function () {
  "use strict";

  const API_BASE = (window.MH_TESTNET_API_BASE || "/testnet-api").replace(/\/$/, "");
  const POOL_BASE = String(window.MH_MESH_POOL || "https://eu.hashmonkeys.cloud").replace(/\/$/, "");
  const SCENARIOS = [
    { id: "block_propagation", label: "Spread blocks", aim: "Make sure new blocks reach everyone quickly" },
    { id: "security_adversary", label: "Catch cheaters", aim: "Spot spam and bad actors early" },
    { id: "privacy_leakage", label: "Protect privacy", aim: "Keep network traffic harder to track" },
    { id: "scale_throughput", label: "Handle growth", aim: "Keep the chain smooth as more people join" },
    { id: "spam_recovery", label: "Recover from spam", aim: "Bounce back after junk traffic" },
    { id: "routing_efficiency", label: "Send work wisely", aim: "Give AI jobs to the right workers" },
    { id: "market_balance", label: "Keep pay fair", aim: "Keep CPU, GPU, and node rewards in balance" },
    { id: "verifier_quorum", label: "Double-check results", aim: "Make sure AI results are checked properly" },
  ];

  /** Trilemma needles + guardian legs — plain-English goals for the Training tab. */
  const LEGS = [
    {
      key: "security",
      needle: "sec",
      trainId: "security",
      title: "Security",
      goal: "Catch spam and cheaters before they hurt the chain",
      how: "Guardian model practices spot-the-bad-actor drills",
    },
    {
      key: "scale",
      needle: "scale",
      trainId: "network",
      title: "Scale",
      goal: "Stay smooth as more people and jobs join",
      how: "Guardian model practices growth / backlog / latency drills",
    },
    {
      key: "decent",
      needle: "decent",
      trainId: "blocks",
      title: "Decentralization",
      goal: "More independent nodes and AI workers — not one hub",
      how: "Score rises when more peers join; block-spread training still runs",
    },
    {
      key: "transpar",
      needle: "transpar",
      trainId: "transpar",
      title: "Transparency",
      goal: "Keep network traffic harder to secretly track",
      how: "Guardian model practices privacy / linkability drills",
    },
  ];

  /** Quantum research legs (Build/26) — goals vs readiness needles. */
  const QUANTUM_LEGS = [
    {
      key: "pqc",
      needle: "pqc",
      trainId: "pqc",
      title: "Post-quantum crypto",
      goal: "Stay ready if classical signatures get weaker under quantum pressure",
      how: "Guardian practices PQC migration / classical-fragility drills",
    },
    {
      key: "grover",
      needle: "grover",
      trainId: "grover",
      title: "PoW vs Grover",
      goal: "Keep proof-of-work fair if search gets a quantum √N speedup",
      how: "Guardian practices Grover-style search / difficulty pressure drills",
    },
    {
      key: "harvest",
      needle: "secrecy",
      trainId: "harvest",
      title: "Long-term secrecy",
      goal: "Limit harvest-now, decrypt-later risk for recorded traffic",
      how: "Guardian practices long-lived secrecy / aging ciphertext drills",
    },
  ];

  var GOAL_BY_ID = {};
  SCENARIOS.forEach(function (s) {
    GOAL_BY_ID[s.id] = s;
  });

  var state = {
    page: null,
    snap: null,
    blocks: [],
    builtPage: null,
  };

  function esc(s) {
    return String(s ?? "")
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/"/g, "&quot;");
  }

  async function api(path) {
    var ctrl = typeof AbortController !== "undefined" ? new AbortController() : null;
    var timer = ctrl
      ? setTimeout(function () {
          try {
            ctrl.abort();
          } catch (_) {}
        }, 25000)
      : null;
    try {
      var r = await fetch(API_BASE + path, {
        cache: "no-store",
        signal: ctrl ? ctrl.signal : undefined,
      });
      if (!r.ok) throw new Error("HTTP " + r.status);
      return await r.json();
    } finally {
      if (timer) clearTimeout(timer);
    }
  }

  async function fetchPool() {
    try {
      var r = await fetch(POOL_BASE + "/v1/poolstats", { cache: "no-store" });
      if (!r.ok) return {};
      return await r.json();
    } catch (_) {
      return {};
    }
  }

  function poolMinerCount(pool) {
    var n = Number(pool && pool.connected_miners);
    return isFinite(n) && n > 0 ? n : 0;
  }

  function liveMinerCount(snap) {
    var seed = ((snap.mining && snap.mining.active_miners) || []).filter(function (m) {
      return m.mining;
    }).length;
    return Math.max(seed, poolMinerCount(snap.pool));
  }

  /** Seed plus unique P2P peers it has pinged. Not a global census. */
  function liveNodeCount(info) {
    if (!info) return "—";
    var ids = {};
    function add(id) {
      if (!id) return;
      ids[String(id)] = true;
    }
    add(info.peer_id);
    var rtts = info.peer_rtts || [];
    for (var i = 0; i < rtts.length; i++) {
      add(rtts[i] && rtts[i].peer_id);
    }
    var unique = Object.keys(ids).length;
    var peers = Number(info.peers);
    var connected = (isFinite(peers) && peers >= 0 ? peers : 0) + (info.peer_id ? 1 : 0);
    var n = Math.max(unique, connected);
    return n > 0 ? n : "—";
  }

  function nodePeerListHtml(info) {
    var ids = [];
    var seen = {};
    function add(id, label) {
      if (!id) return;
      var k = String(id);
      if (seen[k]) return;
      seen[k] = true;
      ids.push({ id: k, label: label || "peer" });
    }
    add(info && info.peer_id, "seed");
    ((info && info.peer_rtts) || []).forEach(function (p) {
      add(p && p.peer_id, p.rtt_ms != null ? p.rtt_ms + " ms" : "peer");
    });
    if (!ids.length) {
      return '<p class="tn-muted">No P2P peers reported by the seed yet.</p>';
    }
    return ids
      .map(function (row) {
        return (
          '<div class="tn-row"><span>' +
          esc(row.label) +
          '</span><span class="tn-mono">' +
          esc(shortHash(row.id)) +
          "</span></div>"
        );
      })
      .join("");
  }

  /** Where miners work: pool template height, else seed. One chain — not two clocks. */
  function mineTip(snap) {
    var poolH = Number(snap.pool && snap.pool.block_height);
    var seedH = Number(snap.info && snap.info.height);
    var p = isFinite(poolH) && poolH > 0 ? poolH : 0;
    var s = isFinite(seedH) && seedH > 0 ? seedH : 0;
    return Math.max(p, s);
  }

  function seedBehindMine(snap) {
    var poolH = Number(snap.pool && snap.pool.block_height);
    var seedH = Number(snap.info && snap.info.height);
    return isFinite(poolH) && isFinite(seedH) && poolH > seedH + 2;
  }

  function minersHtml(snap) {
    var pool = snap.pool || {};
    var credits = pool.credits || {};
    var keys = Object.keys(credits);
    var seed = ((snap.mining && snap.mining.active_miners) || []).filter(function (m) {
      return m.mining;
    });
    if (!keys.length && !seed.length) {
      var n = poolMinerCount(pool);
      if (n > 0) {
        return (
          '<p class="tn-muted">' +
          n +
          " miner" +
          (n === 1 ? "" : "s") +
          " pulling jobs from the HTTPS pool.</p>"
        );
      }
      return '<p class="tn-muted">No miners on the HTTPS pool right now.</p>';
    }
    var rows = [];
    keys.sort(function (a, b) {
      return Number(credits[b] || 0) - Number(credits[a] || 0);
    });
    keys.forEach(function (k) {
      var parts = String(k).split(".");
      var worker = parts.length > 1 ? parts.slice(1).join(".") : "";
      var label = worker || shortAddr(parts[0]);
      rows.push(
        '<div class="tn-row"><span class="tn-mono">' +
          esc(label) +
          "</span><span>" +
          Number(credits[k] || 0).toLocaleString() +
          " blocks · pool</span></div>"
      );
    });
    seed.forEach(function (m) {
      rows.push(
        '<div class="tn-row"><span class="tn-mono">' +
          esc(m.short || shortAddr(m.address)) +
          "</span><span>seed RPC · height " +
          esc(m.height) +
          "</span></div>"
      );
    });
    return rows.join("");
  }

  function pageFromHash() {
    var h = (location.hash || "#overview").slice(1);
    if (h.startsWith("block/")) return { name: "block", id: h.split("/")[1] };
    if (h.startsWith("tx/")) return { name: "tx", id: h.split("/")[1] };
    if (h === "learn" || h === "ai") return { name: "adaptive", id: null };
    if (h.startsWith("addr/")) {
      var raw = h.slice(5);
      try {
        raw = decodeURIComponent(raw);
      } catch (_) {}
      return { name: "utxos", id: raw || null };
    }
    return { name: h || "overview", id: null };
  }

  function setStatus(msg, ok) {
    var el = document.getElementById("tnStatus");
    if (!el) return;
    if (el.dataset.msg === msg && String(el.dataset.ok) === String(ok !== false)) return;
    el.dataset.msg = msg;
    el.dataset.ok = String(ok !== false);
    el.textContent = msg;
    el.style.color = ok === false ? "var(--rose, #fb7185)" : "var(--muted)";
  }

  function setActiveTab(name) {
    document.querySelectorAll("#tnTabs a").forEach(function (a) {
      a.classList.toggle(
        "active",
        a.dataset.page === name ||
          (name === "block" && a.dataset.page === "blocks") ||
          (name === "tx" && a.dataset.page === "txs") ||
          (name === "utxos" && a.dataset.page === "txs") ||
          ((name === "adaptive" || name === "ai") && a.dataset.page === "adaptive")
      );
    });
  }

  function fmtAtomic(atomic) {
    var n = Number(atomic);
    if (!isFinite(n)) return "—";
    return (n / 1e8).toFixed(8).replace(/\.?0+$/, "") + " MESH";
  }

  function sumTxOut(tx) {
    var outs = (tx && tx.outputs) || [];
    var total = 0;
    outs.forEach(function (o) {
      if (o.atomic != null) total += Number(o.atomic) || 0;
      else if (o.amount) {
        var m = String(o.amount).match(/([\d.]+)/);
        if (m) total += Math.round(parseFloat(m[1]) * 1e8);
      }
    });
    return total;
  }

  function pomcLayout(memo) {
    var m = String(memo || "");
    var segs = m.split("|");
    var main = segs[0] || "";
    if (!main.startsWith("pomc:v1:")) return null;
    var bits = main.split(":");
    var exam = null;
    for (var i = 1; i < segs.length; i++) {
      if (segs[i].indexOf("exam:") === 0) {
        var n = parseInt(segs[i].slice(5), 10);
        if (isFinite(n)) exam = n;
      }
    }
    return {
      height: bits[2] || "?",
      nGpu: parseInt(bits[3], 10) || 0,
      nNode: parseInt(bits[4], 10) || 0,
      nExam: exam,
    };
  }

  function pomcExplain(memo) {
    var lay = pomcLayout(memo);
    if (!lay) return null;
    var detail =
      lay.nGpu === 0
        ? "Block #" +
          lay.height +
          " — 90% finder / 10% nodes. One Fusion pay line. 20 confirms to spend."
        : lay.nExam != null
        ? "Block #" +
          lay.height +
          " — 45% Fusion seal, 45% GPU work, 10% nodes. Same finder gets both 45s. 20 confirms to spend."
        : "Block #" +
          lay.height +
          " — CPU / GPU / node coinbase (20 confirms to spend).";
    return { kind: "Block reward (PoMC)", height: lay.height, detail: detail };
  }

  function outRole(idx, count, memo, out) {
    if (out && out.title) return out.title;
    var lay = pomcLayout(memo);
    if (!lay) return "Output #" + idx;
    if (idx === 0) return lay.nGpu === 0 ? "Finder · 90%" : "Fusion seal · 45%";
    if (idx <= lay.nGpu) {
      if (lay.nExam != null) {
        if (idx <= lay.nExam) return "GPU work · helper share";
        return "GPU work · 45%";
      }
      return "GPU work · 45%";
    }
    return "Node work · 10%";
  }

  function lanePillClass(title) {
    var t = String(title || "").toLowerCase();
    if (t.indexOf("exam") >= 0) return "tn-pill tn-pill--exam";
    if (t.indexOf("fusion mix") >= 0 || t.indexOf("gpu") >= 0) return "tn-pill tn-pill--fusion";
    if (t.indexOf("cpu") >= 0 || t.indexOf("find") >= 0) return "tn-pill tn-pill--cpu";
    if (t.indexOf("node") >= 0) return "tn-pill tn-pill--node";
    return "tn-pill";
  }

  function addrLink(addr) {
    var a = String(addr || "");
    if (!a) return "—";
    return (
      '<a class="tn-mono tn-addr-link" href="#addr/' +
      encodeURIComponent(a) +
      '" title="' +
      esc(a) +
      '">' +
      esc(shortAddr(a)) +
      "</a>"
    );
  }

  /** Soft + quantum-gated self-evolution stories (Build/30). */
  function softStories(snap) {
    var env = snap.envelopes || {};
    var soft = env.envelopes || {};
    var retarget = env.retarget || {};
    var gate = env.quantum_gate || (snap.quantum && snap.quantum.self_evolution) || {};
    var consensus = Number(env.consensus_difficulty ?? (snap.info && snap.info.next_difficulty) ?? 0);
    var hint = Number(env.soft_diff_hint ?? (snap.info && snap.info.soft_diff_hint) ?? consensus);
    var bias = Number(soft.suggested_cpu_diff_bias);
    var thresh = Number(soft.soft_adapt_signal_threshold);
    var rounds = soft.soft_benchmark_rounds;
    var minv = soft.min_verifier_weight;
    var stipend = Number(soft.idle_stipend_bps_cap);
    var interval = Number(retarget.interval != null ? retarget.interval : soft.retarget_interval);
    var step = Number(retarget.step != null ? retarget.step : soft.retarget_step);
    var floor = Number(retarget.min_floor != null ? retarget.min_floor : soft.min_difficulty_floor);
    var groverSince = Number(gate.grover_certs_since_retarget_adapt);
    var groverNeed = Number(gate.min_grover_certs_for_retarget || 5);
    var groverTotal = Number(gate.grover_eval_count);
    var stories = [];

    stories.push({
      tone: "note",
      title: "What AI may and may not change",
      body:
        "Fusion finds the block. Optional AI exams and protocol sims are rematched by the seed. Soft knobs may nudge practice intensity only. The 45/45/10 split, opcodes, and the tip stay human-only.",
    });

    if (isFinite(interval) || isFinite(step) || isFinite(floor)) {
      var gateLine =
        isFinite(groverSince) && isFinite(groverNeed)
          ? " Gate: " +
            Math.min(groverSince, groverNeed) +
            "/" +
            groverNeed +
            " new Grover certs since last retarget adapt" +
            (isFinite(groverTotal) ? " (" + groverTotal + " lifetime)." : ".")
          : "";
      stories.push({
        tone: isFinite(floor) && floor >= 8 ? "strict" : "research",
        title: "Difficulty schedule (human-gated retarget)",
        body:
          "Live retarget every " +
          (isFinite(interval) ? interval : "—") +
          " blocks, step ±" +
          (isFinite(step) ? step : "—") +
          " bit(s), min floor " +
          (isFinite(floor) ? floor : "—") +
          ". Consensus difficulty now " +
          consensus +
          "." +
          gateLine,
      });
    }

    if (isFinite(bias) && bias <= -2) {
      stories.push({
        tone: "ease",
        title: "Peak soft performance — make CPU mining easier (hint only)",
        body:
          "Consensus still requires difficulty " +
          consensus +
          ". Soft hint is " +
          hint +
          " (bias " +
          bias +
          "). Aimed at best throughput when the mesh looks healthy — rejected if it fails real consensus.",
      });
    } else if (isFinite(bias) && bias < 0) {
      stories.push({
        tone: "ease",
        title: "Make CPU mining a bit easier (hint only)",
        body:
          "Real blocks still need consensus difficulty " +
          consensus +
          ". Miners see soft hint " +
          hint +
          " (bias " +
          bias +
          "). That can feel slightly easier when picking work — rejected if it fails real consensus.",
      });
    } else if (isFinite(bias) && bias > 0) {
      stories.push({
        tone: "strict",
        title: "Nudge CPU mining a bit harder (hint only)",
        body:
          "Consensus difficulty is " +
          consensus +
          "; soft hint is " +
          hint +
          " (bias +" +
          bias +
          "). Miners may aim slightly harder — validation still uses consensus rules.",
      });
    } else {
      stories.push({
        tone: "neutral",
        title: "Keep CPU soft hint aligned with consensus",
        body:
          "Soft mining hint matches consensus difficulty (" +
          consensus +
          "). No extra ease or squeeze right now.",
      });
    }

    if (isFinite(thresh)) {
      if (thresh <= 0.25) {
        stories.push({
          tone: "research",
          title: "Start extra GPU practice sooner",
          body:
            "When AI progress looks weak (signal under " +
            thresh +
            "), the seed queues more GPU benchmark jobs earlier.",
        });
      } else if (thresh >= 0.6) {
        stories.push({
          tone: "research",
          title: "Wait longer before extra GPU practice",
          body:
            "Only enqueue extra benchmarks when the progress signal drops under " +
            thresh +
            " — cooler research cadence.",
        });
      } else {
        stories.push({
          tone: "research",
          title: "Balanced trigger for extra GPU practice",
          body:
            "Extra benchmark jobs fire when the progress signal dips under " + thresh + ".",
        });
      }
    }

    if (rounds != null) {
      stories.push({
        tone: "research",
        title: "Each practice job ≈ " + rounds + " rounds",
        body:
          "Bigger numbers mean longer GPU workouts when research is intense (also scaled by the research budget cap).",
      });
    }

    if (minv != null) {
      stories.push({
        tone: Number(minv) >= 3 ? "strict" : "neutral",
        title:
          Number(minv) >= 3
            ? "Be pickier about which AI results count"
            : "Keep a moderate bar for AI result checks",
        body:
          "Verifier weight floor is " +
          minv +
          ". Slow or weak workers get less credit; results need stronger checks before they move settings.",
      });
    }

    if (isFinite(stipend)) {
      if (stipend >= 1000) {
        stories.push({
          tone: "research",
          title: "Research / relay budget at full soft cap",
          body:
            "Idle stipend cap is " +
            stipend +
            " — node relay credits and research intensity can run at the soft maximum.",
        });
      } else {
        stories.push({
          tone: "research",
          title: "Throttle research / relay intensity",
          body:
            "Idle stipend cap is " +
            stipend +
            " (below 1000) — scales down node credits and how hard research workouts run.",
        });
      }
    }

    return stories;
  }

  function humanizeRationale(text) {
    var raw = String(text || "").trim();
    if (!raw) return "";
    return raw
      .split(/;\s*/)
      .map(function (part) {
        var p = part.trim();
        if (!p) return "";
        if (/quantum-gated-retarget/i.test(p))
          return "Quantum Grover certificates unlocked a bounded difficulty-schedule change";
        if (/quantum_grover weak/i.test(p))
          return "Grover pressure-tests look weak → harden min difficulty floor / retarget step";
        if (/quantum_grover healthy/i.test(p))
          return "Grover looks healthy → allow a tighter retarget interval (still clamped)";
        if (/quantum pressure-tests weak/i.test(p))
          return "Quantum readiness weak → more quantum guardian training";
        if (/quantum readiness/i.test(p)) return p;
        if (/high GPU backlog/i.test(p)) return "GPU queue is busy → cool down how often we ask for more practice";
        if (/high latency/i.test(p)) return "Workers feel slow → raise the bar for trusted AI results";
        if (/security sim weak/i.test(p)) return "Security drills look weak → insist on stronger verification";
        if (/scale backlog high/i.test(p)) return "Scale pressure is high → spend more GPU research budget";
        if (/low mean research/i.test(p)) return "Research scores look soft → intensify protocol practice";
        if (/sparse research history/i.test(p)) return "Not enough research history yet → raise research budget";
        if (/growth stress/i.test(p)) return "Growth stress (orphans / backlog) → stricter checks + softer CPU hint";
        if (/protocol_eval/i.test(p)) return "Checked protocol evaluations are feeding auto-adapt";
        if (/auto-adapt:/i.test(p)) return p.replace(/^auto-adapt:\s*/i, "Why it moved: ");
        return p;
      })
      .filter(Boolean)
      .join(". ");
  }

  function finalityLabel(info) {
    if (info && info.finality_active) {
      var h = info.finalized_height;
      if (h) return "#" + h + " · " + shortHash(info.finalized_hash);
      return "active · none locked";
    }
    return "off (lab)";
  }

  function shortHash(h) {
    var s = String(h || "");
    if (s.length <= 16) return s || "—";
    return s.slice(0, 10) + "…" + s.slice(-6);
  }

  function shortAddr(a) {
    var s = String(a || "");
    if (s.length <= 18) return s || "—";
    return s.slice(0, 10) + "…" + s.slice(-6);
  }

  function fmtMeshInt(v, withUnit) {
    if (v == null || v === "") return "";
    var n = parseFloat(String(v).replace(/[^0-9.]/g, ""));
    if (!isFinite(n)) return String(v);
    var s = Math.round(n).toLocaleString();
    return withUnit === false ? s : s + " MESH";
  }

  var SUPPLY_CAP_MESH = 2522880000;
  var MESH_ATOMIC = 100000000;

  function supplyFromSnap(snap) {
    var info = (snap && snap.info) || {};
    var markets = (snap && snap.markets) || {};
    var cap = Number(info.supply_cap_mesh || markets.supply_cap_mesh || SUPPLY_CAP_MESH);
    if (!isFinite(cap) || cap <= 0) cap = SUPPLY_CAP_MESH;
    var raw = info.emitted_atomic != null ? info.emitted_atomic : markets.emitted_atomic;
    var emitted = NaN;
    if (raw != null && String(raw) !== "") {
      var a = Number(raw);
      if (isFinite(a)) emitted = a / MESH_ATOMIC;
    }
    if (!isFinite(emitted)) {
      var nBlocks = Number(
        info.blocks != null ? info.blocks : info.height != null ? Number(info.height) + 1 : 0
      );
      emitted = nBlocks > 0 ? nBlocks * 50 : 0;
    }
    return {
      cap: cap,
      emitted: emitted,
      remain: Math.max(0, cap - emitted),
      pct: cap > 0 ? (emitted / cap) * 100 : 0,
    };
  }

  function fmtAge(ts) {
    if (ts == null || ts === 0) return "—";
    var n = Number(ts);
    var ms = n < 1e12 ? n * 1000 : n;
    var sec = Math.max(0, Math.round((Date.now() - ms) / 1000));
    if (sec < 60) return sec + "s ago";
    if (sec < 3600) return Math.floor(sec / 60) + "m ago";
    return Math.floor(sec / 3600) + "h ago";
  }

  function fmtTime(ts) {
    if (ts == null || ts === 0) return "—";
    var n = Number(ts);
    var ms = n < 1e12 ? n * 1000 : n;
    try {
      return new Date(ms).toISOString().replace("T", " ").replace(/\.\d+Z$/, " UTC");
    } catch (_) {
      return String(ts);
    }
  }

  function pct01(x) {
    var n = Number(x);
    if (!isFinite(n)) return 0;
    if (n > 1) return Math.max(0, Math.min(100, n));
    return Math.max(0, Math.min(100, n * 100));
  }

  function setText(sel, text, root) {
    var el = (root || document).querySelector(sel);
    if (!el) return;
    var next = String(text ?? "—");
    if (el.textContent === next) return;
    el.textContent = next;
    el.classList.remove("tn-flash");
    void el.offsetWidth;
    el.classList.add("tn-flash");
  }

  function setHTML(sel, html, root) {
    var el = (root || document).querySelector(sel);
    if (!el) return;
    if (el.dataset.html === html) return;
    el.dataset.html = html;
    el.innerHTML = html;
  }

  function setWidth(sel, pct, root) {
    var el = (root || document).querySelector(sel);
    if (!el) return;
    var w = Math.max(0, Math.min(100, Number(pct) || 0)).toFixed(1) + "%";
    if (el.style.width === w) return;
    el.style.width = w;
  }

  function setRing(sel, pct, root) {
    var el = (root || document).querySelector(sel);
    if (!el) return;
    var p = Math.max(0, Math.min(100, Number(pct) || 0));
    var r = 30;
    var c = 2 * Math.PI * r;
    var dash = (p / 100) * c + " " + c;
    if (el.getAttribute("stroke-dasharray") === dash) return;
    el.setAttribute("stroke-dasharray", dash);
  }

  function setClass(sel, cls, on, root) {
    var el = (root || document).querySelector(sel);
    if (!el) return;
    el.classList.toggle(cls, !!on);
  }

  async function fetchSnapshot() {
    var pair = await Promise.all([
      api("/v1/getnodeinfo"),
      api("/v1/markets").catch(function () { return {}; }),
      api("/v1/meshpulse").catch(function () { return {}; }),
      api("/v1/envelopes").catch(function () { return {}; }),
      api("/v1/miningstatus").catch(function () { return { active_miners: [], events: [] }; }),
      api("/v1/ai/health").catch(function () { return {}; }),
      api("/v1/research/status").catch(function () { return {}; }),
      api("/v1/trilemma").catch(function () { return {}; }),
      api("/v1/quantum").catch(function () { return {}; }),
      api("/v1/exam/status").catch(function () { return {}; }),
      api("/v1/result/pending").catch(function () { return {}; }),
      fetchPool(),
    ]);
    var info = pair[0] || {};
    var h = info.height;
    var lastBlock = null;
    if (h != null) {
      lastBlock = await api("/v1/getblock?height=" + encodeURIComponent(h)).catch(function () {
        return null;
      });
    }
    return {
      info: info,
      markets: pair[1],
      pulse: pair[2],
      envelopes: pair[3],
      mining: pair[4],
      ai: pair[5],
      research: pair[6],
      trilemma: pair[7],
      quantum: pair[8],
      exam: pair[9],
      seals: pair[10],
      pool: pair[11],
      lastBlock: lastBlock,
    };
  }

  /** Prefer shared-brain v2 (Q16). v1 stays epoch 0 on Windows miners. */
  function liveBrain(snap) {
    var ai = snap.ai || {};
    var v2 = ai.brain_v2 || {};
    var v1 = ai.brain || {};
    var pulse = snap.pulse || {};
    if (v2.epoch != null || v2.advances || v2.train_steps_total) {
      var accQ = Number(v2.last_acc_q16);
      return {
        ver: 2,
        epoch: Number(v2.epoch || 0),
        advances: Number(v2.advances || 0),
        steps: v2.train_steps_total,
        acc: isFinite(accQ) ? accQ / 65536 : null,
        contract: v2.contract || "v2.0.0",
      };
    }
    var acc = pulse.brain_acc != null ? Number(pulse.brain_acc) : Number(v1.last_acc);
    return {
      ver: 1,
      epoch: Number(pulse.brain_epoch != null ? pulse.brain_epoch : v1.epoch || 0),
      advances: Number(pulse.brain_advances || v1.advances || 0),
      steps: v1.train_steps_total,
      acc: isFinite(acc) ? acc : null,
      contract: "v1",
    };
  }

  function needleTone(score) {
    var n = Number(score);
    if (!isFinite(n)) return "muted";
    if (n >= 70) return "ok";
    if (n >= 40) return "warn";
    return "bad";
  }

  function trilemmaBoard(snap) {
    var t = snap.trilemma || {};
    return t.board || (snap.pulse && snap.pulse.trilemma) || {};
  }

  function legTrainMap(snap) {
    var map = {};
    var legs = (snap.trilemma && snap.trilemma.legs) || [];
    legs.forEach(function (L) {
      if (L && L.leg) map[L.leg] = L;
    });
    return map;
  }

  function weakestKey(board) {
    var w = String(board.weakest || "").toLowerCase();
    if (w === "network" || w === "scale") return "scale";
    if (w === "security" || w === "sec") return "security";
    if (w === "transpar" || w === "transparency") return "transpar";
    if (w === "decent" || w === "decentralization") return "decent";
    return w || "decent";
  }

  function brainGoalCard(snap) {
    var pulse = snap.pulse || {};
    var ai = snap.ai || {};
    var brain = liveBrain(snap);
    var epoch = brain.epoch;
    var acc = brain.acc;
    var steps = brain.steps;
    var dataset = ai.ml_dataset || "Shared digit model (MNIST)";
    var accPct = acc != null && isFinite(acc) ? (acc * 100).toFixed(1) + "%" : "—";
    var goalMet = isFinite(acc) && acc >= 0.95;
    return {
      title: "Shared brain",
      goal: "Teach one shared digit model so every miner improves the same network AI",
      now:
        "Epoch " +
        epoch +
        " · accuracy " +
        accPct +
        (steps != null ? " · " + Number(steps).toLocaleString() + " train steps" : ""),
      dataset: dataset,
      tone: goalMet ? "ok" : isFinite(acc) && acc >= 0.7 ? "warn" : "muted",
      progress: isFinite(acc) ? Math.round(acc * 100) : 0,
      targetHint: "Goal: keep accuracy high (near 100%) while the model keeps learning",
    };
  }

  function legsGoalCards(snap) {
    var board = trilemmaBoard(snap);
    var trains = legTrainMap(snap);
    var weak = weakestKey(board);
    return LEGS.map(function (leg) {
      var score = board[leg.needle];
      var train = trains[leg.trainId] || {};
      var epochs =
        (board.leg_epochs && board.leg_epochs[leg.trainId]) != null
          ? board.leg_epochs[leg.trainId]
          : train.epoch != null
            ? train.epoch
            : 0;
      var smart =
        (board.leg_smart && board.leg_smart[leg.trainId]) != null
          ? board.leg_smart[leg.trainId]
          : train.last_acc != null
            ? Math.round(Number(train.last_acc) * 100)
            : null;
      var isFocus = weak === leg.key;
      var scoreN = score != null ? Number(score) : null;
      return {
        key: leg.key,
        title: leg.title,
        goal: leg.goal,
        how: leg.how,
        score: scoreN,
        epochs: epochs,
        smart: smart,
        focus: isFocus,
        tone: needleTone(scoreN),
        nowLine:
          (scoreN != null ? scoreN + " / 100 on the public needle" : "No score yet") +
          " · trained " +
          Number(epochs || 0).toLocaleString() +
          " epochs" +
          (smart != null ? " · guardian smart " + smart + "%" : ""),
      };
    });
  }

  function focusStory(snap) {
    var board = trilemmaBoard(snap);
    var weak = weakestKey(board);
    var leg = LEGS.find(function (L) {
      return L.key === weak;
    });
    var bal = board.balance != null ? board.balance : "—";
    var q = quantumBoard(snap);
    var qStory = (snap.quantum && snap.quantum.story) || {};
    var qLine = "";
    if (qStory.headline) {
      qLine = " " + qStory.headline + ".";
    } else if (q && (q.pqc != null || q.readiness != null)) {
      qLine =
        " Quantum readiness " +
        (q.readiness != null ? q.readiness : "—") +
        "/100 (weakest: " +
        (q.weakest || "—") +
        ").";
    }
    if (!leg) {
      return "Training feeds the weakest network leg first. Balance score: " + bal + "/100." + qLine;
    }
    return (
      "Focus now: " +
      leg.title +
      " (weakest Trilemma leg). Goal — " +
      leg.goal +
      ". Balance across legs: " +
      bal +
      "/100 (100 = even)." +
      qLine
    );
  }

  function quantumBoard(snap) {
    var q = snap.quantum || {};
    return q.board || {};
  }

  function quantumTrainMap(snap) {
    var map = {};
    var legs = (snap.quantum && snap.quantum.legs) || [];
    legs.forEach(function (L) {
      if (L && L.leg) map[L.leg] = L;
    });
    return map;
  }

  function quantumWeakestKey(board) {
    var w = String(board.weakest || "").toLowerCase();
    if (w === "secrecy" || w === "harvest") return "harvest";
    if (w === "pqc" || w === "post_quantum") return "pqc";
    if (w === "grover") return "grover";
    return w || "pqc";
  }

  function quantumGoalCards(snap) {
    var board = quantumBoard(snap);
    var trains = quantumTrainMap(snap);
    var hasBoard = board && (board.pqc != null || board.grover != null || board.secrecy != null);
    var weak = quantumWeakestKey(board);
    return QUANTUM_LEGS.map(function (leg) {
      var score = board[leg.needle];
      var train = trains[leg.trainId] || {};
      var epochs =
        (board.leg_epochs && board.leg_epochs[leg.trainId]) != null
          ? board.leg_epochs[leg.trainId]
          : train.epoch != null
            ? train.epoch
            : 0;
      var smart =
        (board.leg_smart && board.leg_smart[leg.trainId]) != null
          ? board.leg_smart[leg.trainId]
          : train.last_acc != null
            ? Math.round(Number(train.last_acc) * 100)
            : null;
      var scoreN = score != null ? Number(score) : null;
      return {
        key: leg.key,
        title: leg.title,
        goal: leg.goal,
        how: leg.how,
        score: scoreN,
        epochs: epochs,
        smart: smart,
        focus: hasBoard && weak === leg.key,
        tone: hasBoard ? needleTone(scoreN) : "muted",
        available: hasBoard,
        nowLine: !hasBoard
          ? "Waiting for seed Quantum guardians (Build/26)…"
          : (scoreN != null ? scoreN + " / 100 on the public needle" : "No score yet") +
            " · trained " +
            Number(epochs || 0).toLocaleString() +
            " epochs" +
            (smart != null ? " · guardian smart " + smart + "%" : ""),
      };
    });
  }

  function gradeHighBetter(v) {
    if (v == null || !isFinite(Number(v))) return { label: "No data yet", tone: "muted" };
    var n = Number(v);
    if (n >= 0.7) return { label: "Good", tone: "ok" };
    if (n >= 0.4) return { label: "Okay", tone: "warn" };
    return { label: "Needs work", tone: "bad" };
  }

  function gradeLowBetter(v) {
    if (v == null || !isFinite(Number(v))) return { label: "No data yet", tone: "muted" };
    var n = Number(v);
    if (n <= 0.3) return { label: "Good", tone: "ok" };
    if (n <= 0.55) return { label: "Okay", tone: "warn" };
    return { label: "Needs work", tone: "bad" };
  }

  /** Pick the plain-English goal the network should study next (mirrors node AI tick). */
  function pickAim(snap) {
    var qStory = (snap.quantum && snap.quantum.story) || {};
    var qBoard = quantumBoard(snap);
    if (qBoard.readiness != null && Number(qBoard.readiness) < 55 && qBoard.weakest) {
      var qid =
        qBoard.weakest === "harvest"
          ? "quantum_harvest"
          : qBoard.weakest === "grover"
            ? "quantum_grover"
            : "quantum_pqc";
      if (GOAL_BY_ID[qid]) return GOAL_BY_ID[qid];
    }
    if (qStory.focus) {
      var fid =
        qStory.focus === "harvest"
          ? "quantum_harvest"
          : qStory.focus === "grover"
            ? "quantum_grover"
            : qStory.focus === "pqc"
              ? "quantum_pqc"
              : null;
      var act = (snap.quantum && snap.quantum.activity) || [];
      if (fid && GOAL_BY_ID[fid] && act.length) return GOAL_BY_ID[fid];
    }
    var pulse = snap.pulse || {};
    var mkt = pulse.markets || {};
    var scores = mkt.research_scores || {};
    var signal = Number(pulse.gpu_vs_height_signal || 0);
    var avgLat = Number(mkt.avg_latency_ms || 0);
    var orphan = Number(scores.mean_orphan_risk || 0);
    var detect = Number(scores.mean_detect_rate || 0);
    var backlog = Number(scores.mean_backlog_ratio || 0);
    var progress = Number(mkt.research_progress || 0);
    var primary = Number(scores.mean_primary || 0);
    var id = "privacy_leakage";
    if (orphan > 0.45) id = "block_propagation";
    else if (detect > 0 && detect < 0.65) id = "security_adversary";
    else if (backlog > 0.55 || avgLat > 1500) id = "scale_throughput";
    else if (signal < 0.25) id = "routing_efficiency";
    else if (progress < 0.25) id = "market_balance";
    else if (primary > 0 && primary < 0.45) id = "verifier_quorum";
    return GOAL_BY_ID[id] || SCENARIOS[0];
  }

  function aiDoingLine(snap) {
    var ai = snap.ai || {};
    var pulse = snap.pulse || {};
    var mkt = pulse.markets || {};
    var gpuW = Number(mkt.pending_gpu_weight || 0);
    var verifyOk = Number(ai.verify_ok != null ? ai.verify_ok : (snap.research && snap.research.verify_ok) || 0);
    var pending = Number(ai.pending || 0);
    var inflight = Number(ai.inflight || 0);
    var br = liveBrain(snap);
    var brainEpoch = br.epoch;
    var advances = br.advances;
    var qStory = (snap.quantum && snap.quantum.story) || {};
    var qAct = (snap.quantum && snap.quantum.activity) || [];

    if (verifyOk === 0 && gpuW === 0 && pending === 0 && inflight === 0) {
      return {
        status: "Idle",
        detail: "No AI jobs in flight. Turn on AI research in the Miner if you want protocol sims alongside Fusion MeshHash.",
        live: false,
      };
    }
    if (inflight > 0 || pending > 0) {
      return {
        status: "Jobs running",
        detail:
          "In flight " +
          inflight +
          " · queued " +
          pending +
          " · verified " +
          verifyOk +
          (gpuW ? " · pending weight " + gpuW : "") +
          ".",
        live: true,
      };
    }
    return {
      status: "Active",
      detail: verifyOk + " AI jobs verified" + (advances ? " · " + advances + " brain advances" : "") + (brainEpoch ? " · brain epoch " + brainEpoch : "") + ".",
      live: true,
    };
  }

  function easyResults(snap) {
    var ai = snap.ai || {};
    var pulse = snap.pulse || {};
    var mkt = pulse.markets || {};
    var scores = mkt.research_scores || {};
    var verifyOk = Number(ai.verify_ok != null ? ai.verify_ok : (snap.research && snap.research.verify_ok) || 0);
    var protocolOk = Number((snap.research && snap.research.protocol_eval_ok) || mkt.research_eval_receipts || 0);
    var br = liveBrain(snap);
    var brainAdvances = br.advances;
    var trainOk = Math.max(0, verifyOk - protocolOk);
    var gpuW = Number(mkt.pending_gpu_weight || 0);
    var brainEpoch = br.epoch;
    var acc = br.acc;
    return [
      {
        title: "AI jobs checked",
        value: String(verifyOk),
        hint: protocolOk + " protocol sims · " + trainOk + " other research",
        tone: verifyOk > 0 ? "ok" : "muted",
      },
      {
        title: "In flight / queued",
        value: Number(ai.inflight || 0) + " / " + Number(ai.pending || 0),
        hint: gpuW ? "Pending GPU weight " + gpuW : "Miner AI research when enabled",
        tone: Number(ai.inflight || 0) > 0 ? "ok" : "muted",
      },
      {
        title: "Shared brain",
        value: brainEpoch > 0 ? "epoch " + brainEpoch : "not training",
        hint:
          brainEpoch > 0 && acc != null && isFinite(acc)
            ? "v" + br.ver + " · accuracy ~" + (acc * 100).toFixed(1) + "%"
            : "Optional — not required for Fusion blocks",
        tone: brainEpoch > 0 || brainAdvances > 0 ? "ok" : "muted",
      },
    ];
  }

  function pipeState(snap) {
    var mkt = (snap.pulse && snap.pulse.markets) || {};
    var env = snap.envelopes || {};
    var ai = snap.ai || {};
    var gpuW = Number(mkt.pending_gpu_weight || 0);
    var research = Number(mkt.research_eval_receipts || 0);
    var verifyOk = Number(ai.verify_ok || 0);
    var hasEpoch = !!env.latest_epoch || Number(env.param_epoch || 0) > 0;
    var working = Number(ai.inflight || 0) > 0 || Number(ai.pending || 0) > 0;
    return [
      {
        key: "gpu",
        title: "1. Workers join",
        detail: gpuW > 0 || verifyOk > 0 ? "People are running AI work" : "Waiting for miners with AI on",
        active: gpuW > 0 || working,
        done: gpuW > 0 || verifyOk > 0,
      },
      {
        key: "sim",
        title: "2. AI studies the chain",
        detail: working ? "Running tests + training now" : verifyOk > 0 ? "Jobs completed" : "Idle",
        active: working,
        done: verifyOk > 0,
      },
      {
        key: "verify",
        title: "3. Results get checked",
        detail: research + " blockchain tests verified",
        active: research > 0 && !hasEpoch,
        done: research > 0,
      },
      {
        key: "apply",
        title: "4. Settings / schedule update",
        detail: hasEpoch
          ? "Version v" +
            (env.param_epoch ?? "0") +
            (env.retarget
              ? " · retarget " +
                (env.retarget.interval ?? "—") +
                "/" +
                (env.retarget.step ?? "—") +
                " floor " +
                (env.retarget.min_floor ?? "—")
              : "")
          : "Not yet — needs more findings",
        active: hasEpoch,
        done: hasEpoch,
      },
    ];
  }

  async function fetchRecentBlocks(height, limit) {
    var hMax = Number(height || 0);
    var start = Math.max(0, hMax - (limit - 1));
    var heights = [];
    for (var h = hMax; h >= start; h--) heights.push(h);
    var blocks = [];
    var chunk = 8;
    for (var i = 0; i < heights.length; i += chunk) {
      var slice = heights.slice(i, i + chunk);
      var part = await Promise.all(
        slice.map(function (hh) {
          return api("/v1/getblock?height=" + hh).catch(function () { return null; });
        })
      );
      part.forEach(function (b) { if (b) blocks.push(b); });
    }
    return blocks;
  }

  function ensureHero(snap) {
    var el = document.getElementById("tnHeroStats");
    if (!el) return;
    if (!el.dataset.ready) {
      el.innerHTML =
        '<div class="stat-card" data-hero="height"><small>Height</small><strong class="stat-card__value--num" data-v></strong><span class="stat-card__hint" data-h></span></div>' +
        '<div class="stat-card" data-hero="finality"><small>Finality</small><strong class="stat-card__value--num" data-v></strong><span class="stat-card__hint" data-h></span></div>' +
        '<div class="stat-card" data-hero="last"><small>Last block</small><strong class="stat-card__value--num" data-v></strong><span class="stat-card__hint">Time since find</span></div>' +
        '<div class="stat-card" data-hero="diff"><small>Difficulty</small><strong class="stat-card__value--num" data-v></strong><span class="stat-card__hint">Consensus</span></div>' +
        '<div class="stat-card" data-hero="reward"><small>Block reward</small><strong class="stat-card__value--num" data-v></strong><span class="stat-card__hint">MESH · 45 you + 5 nodes</span></div>' +
        '<div class="stat-card" data-hero="miners"><small>Miners</small><strong class="stat-card__value--num" data-v></strong><span class="stat-card__hint">HTTPS pool</span></div>' +
        '<div class="stat-card" data-hero="nodes"><small>Nodes</small><strong class="stat-card__value--num" data-v></strong><span class="stat-card__hint">Active on P2P</span></div>' +
        '<div class="stat-card" data-hero="emitted"><small>Total emitted</small><strong class="stat-card__value--num" data-v></strong><span class="stat-card__hint">MESH · live issuance</span></div>' +
        '<div class="stat-card" data-hero="remain"><small>Remaining</small><strong class="stat-card__value--num" data-v></strong><span class="stat-card__hint">MESH left to mint</span></div>' +
        '<div class="stat-card" data-hero="cap"><small>Max supply</small><strong class="stat-card__value--num" data-v></strong><span class="stat-card__hint">Hard cap · enforced</span></div>';
      el.dataset.ready = "1";
      el.hidden = false;
      el.removeAttribute("aria-hidden");
    }
    var info = snap.info || {};
    var markets = snap.markets || {};
    var tip = mineTip(snap);
    setText('[data-hero="height"] [data-v]', tip || info.height || "—", el);
    setText(
      '[data-hero="height"] [data-h]',
      seedBehindMine(snap)
        ? "Mine tip · seed syncing #" + (info.height ?? "?")
        : "One chain · ~5s target",
      el
    );
    setText('[data-hero="finality"] [data-v]', finalityLabel(info), el);
    setText(
      '[data-hero="finality"] [data-h]',
      info.finality_active
        ? "Bonded attest · ~1000-block window"
        : "20 confirms spendable · F2 off",
      el
    );
    var last = snap.lastBlock || {};
    var poolBlk = ((snap.pool && snap.pool.recent_blocks) || [])[0] || {};
    var lastAge = seedBehindMine(snap) && poolBlk.created
      ? fmtAge(poolBlk.created)
      : last.timestamp
        ? fmtAge(last.timestamp)
        : "—";
    setText('[data-hero="last"] [data-v]', lastAge, el);
    setText('[data-hero="diff"] [data-v]', info.next_difficulty ?? "—", el);
    setText('[data-hero="reward"] [data-v]', fmtMeshInt(markets.block_reward, false) || "50", el);
    setText('[data-hero="miners"] [data-v]', liveMinerCount(snap), el);
    setText('[data-hero="nodes"] [data-v]', liveNodeCount(info), el);
    var supply = supplyFromSnap(snap);
    setText('[data-hero="emitted"] [data-v]', fmtMeshInt(supply.emitted, false) || "—", el);
    setText('[data-hero="remain"] [data-v]', fmtMeshInt(supply.remain, false) || "—", el);
    setText('[data-hero="cap"] [data-v]', "2.52B", el);
  }

  function aiPanelHtml() {
    return (
      '<div class="tn-card tn-ai" data-panel="ai">' +
      '<p class="tn-ai__headline"><span class="tn-status-dot" data-ai-dot></span>AI jobs</p>' +
      '<div class="tn-ai-status">' +
      '<div class="tn-ai-status__badge" data-ai-badge>…</div>' +
      '<p class="tn-ai__story" data-ai-story></p>' +
      "</div>" +
      '<div class="tn-easy" data-easy-results></div>' +
      '<div class="tn-easy" data-exam-tape></div>' +
      '<p class="tn-muted tn-ai__foot">Fusion finds the block. From height 39000 the finder must MATCH the immune exam to submit. Helper-floor MESH pays rematched exams and brain steps. Research cannot move BPS.</p>' +
      "</div>"
    );
  }

  function updateAiPanel(root, snap) {
    var panel = root.querySelector('[data-panel="ai"]');
    if (!panel) return;
    var doing = aiDoingLine(snap);
    var results = easyResults(snap);
    setText("[data-ai-badge]", doing.status, panel);
    setText("[data-ai-story]", doing.detail, panel);
    setClass("[data-ai-dot]", "is-live", doing.live, panel);
    setClass("[data-ai-badge]", "is-live", doing.live, panel);
    var easyHtml = results
      .map(function (r) {
        return (
          '<div class="tn-easy__card tone-' +
          esc(r.tone) +
          '"><small>' +
          esc(r.title) +
          "</small><strong>" +
          esc(r.value) +
          '</strong><span class="tn-muted">' +
          esc(r.hint) +
          "</span></div>"
        );
      })
      .join("");
    setHTML("[data-easy-results]", easyHtml, panel);
    var tape = ((snap.exam && snap.exam.recent) || []).slice(0, 8);
    var tapeHtml = tape
      .map(function (e) {
        return (
          '<div class="tn-easy__card tone-ok"><small>' +
          esc(e.title || e.scenario || "exam") +
          "</small><strong>MATCH</strong><span class=\"tn-muted\">" +
          esc((e.worker || "").slice(0, 14)) +
          " · " +
          esc(String(e.latency_ms || 0)) +
          " ms rematch</span></div>"
        );
      })
      .join("");
    setHTML("[data-exam-tape]", tapeHtml, panel);
  }

  function buildOverviewShell(root) {
    root.innerHTML =
      '<div class="tn-card">' +
      '<h3>What this chain is doing</h3>' +
      '<p class="tn-lead"><b>Why mine it?</b> Your PC’s CPU and GPU find one Fusion block together. You get 90%. Nodes that actually help get 10%. Optional tiny AI jobs are extra checks — they do not find the block.</p>' +
      '<p class="tn-lead">MonkeyMesh is a <b>home-PC coin</b>. First valid nonce wins. Paid to the wallet in the miner — not a pool treasury. The 90 / 10 pay line starts at height <b>50000</b> (until then the same finder still gets 45 MESH as two 22.5s).</p>' +
      '<div class="tn-split" aria-hidden="true"><span class="tn-split__cpu"></span><span class="tn-split__gpu"></span><span class="tn-split__node"></span></div>' +
      '<div class="tn-split-legend"><span><b>90% finder</b></span><span><b>10% nodes</b></span></div>' +
      '<div class="tn-steps">' +
      '<div class="tn-step"><small>1</small><strong>Template</strong><span>Miner pulls work from the HTTPS pool.</span></div>' +
      '<div class="tn-step"><small>2</small><strong>Fusion hash</strong><span>CPU fills the pad. GPU mixes in VRAM. One digest.</span></div>' +
      '<div class="tn-step"><small>3</small><strong>Submit</strong><span>Pool / seed rematch the hash. Fake work is rejected.</span></div>' +
      '<div class="tn-step"><small>4</small><strong>Pay</strong><span>Finder 90% · nodes 10%. Spend after 20 confirms.</span></div>' +
      "</div></div>" +
      '<div class="tn-grid">' +
      '<div class="tn-card"><h3>Live chain</h3>' +
      rowSlot("height", "Height") +
      rowSlot("nnodes", "Active nodes") +
      rowSlot("finality", "Finality") +
      rowSlot("blocks", "Blocks") +
      rowSlot("diff", "Difficulty") +
      rowSlot("mempool", "Mempool") +
      rowSlot("tip", "Tip", true) +
      rowSlot("genesis", "Genesis", true) +
      "</div>" +
      '<div class="tn-card"><h3>This block pays</h3>' +
      rowSlot("cpu", "Finder") +
      rowSlot("gpu", "GPU lane (legacy)") +
      rowSlot("node", "Network nodes") +
      rowSlot("split", "Pay line") +
      '<p class="tn-muted" style="margin-top:.5rem">From height 50000 the finder is one 45 MESH output. Nodes get 5 only for attested work. AI exams do not move this split.</p></div>' +
      "</div>" +
      '<div class="tn-grid">' +
      '<div class="tn-card"><h3>If you are a miner</h3>' +
      '<div class="tn-row"><span>You earn</span><span>45 MESH (90%) to the miner wallet</span></div>' +
      '<div class="tn-row"><span>Nodes earn</span><span>5 MESH per block for attested work</span></div>' +
      '<div class="tn-row"><span>Spend after</span><span>20 confirmations (~100 s)</span></div>' +
      '<div class="tn-row"><span>Target pace</span><span>~5 seconds per block</span></div>' +
      '<p class="tn-muted" style="margin-top:.5rem">No invented network H/s. Point the miner at <span class="tn-mono">https://eu.hashmonkeys.cloud</span> and watch the dashboard.</p></div>' +
      '<div class="tn-card"><h3>Miners connected</h3><div data-miners></div></div>' +
      '<div class="tn-card"><h3>AI in one line</h3>' +
      rowSlot("brain", "Shared brain") +
      rowSlot("receipts", "Jobs rematched") +
      rowSlot("queue", "In flight / queued") +
      '<p class="tn-muted" style="margin-top:.5rem"><a href="#adaptive">See what AI is for</a> — exams and the shared brain. Not a marketplace.</p></div>' +
      "</div>";
  }

  function rowSlot(id, label, mono) {
    return (
      '<div class="tn-row" data-row="' +
      id +
      '"><span>' +
      esc(label) +
      '</span><span' +
      (mono ? ' class="tn-mono"' : "") +
      " data-v>—</span></div>"
    );
  }

  function updateOverview(root, snap) {
    var info = snap.info || {};
    var markets = snap.markets || {};
    var ai = snap.ai || {};
    var br = liveBrain(snap);
    setText('[data-row="height"] [data-v]', info.height ?? "—", root);
    setText('[data-row="nnodes"] [data-v]', liveNodeCount(info), root);
    setText('[data-row="finality"] [data-v]', finalityLabel(info), root);
    setText('[data-row="blocks"] [data-v]', info.blocks ?? "—", root);
    setText('[data-row="diff"] [data-v]', info.next_difficulty ?? "—", root);
    setText('[data-row="mempool"] [data-v]', info.mempool ?? "—", root);
    setText('[data-row="tip"] [data-v]', shortHash(info.tip), root);
    setText('[data-row="genesis"] [data-v]', shortHash(info.genesis), root);
    setText('[data-row="cpu"] [data-v]', markets.cpu_market || "22.5 MESH", root);
    setText('[data-row="gpu"] [data-v]', markets.gpu_market || "22.5 MESH", root);
    setText('[data-row="node"] [data-v]', markets.node_market || "5 MESH", root);
    var split = markets.finder_unify
      ? "90 / 10 from #50000"
      : markets.fair_split
      ? "45 / 45 / 10 until #50000"
      : (markets.cpu_bps ? String(markets.cpu_bps / 100) + "% · unit share" : "45 / 45 / 10");
    setText('[data-row="split"] [data-v]', split, root);
    var aiJobs = ai.verify_ok != null ? ai.verify_ok : 0;
    setText('[data-row="receipts"] [data-v]', aiJobs, root);
    setText(
      '[data-row="brain"] [data-v]',
      br.epoch > 0 ? "v" + br.ver + " · epoch " + br.epoch : "idle",
      root
    );
    setText(
      '[data-row="queue"] [data-v]',
      Number(ai.inflight || 0) + " / " + Number(ai.pending || 0),
      root
    );

    setHTML("[data-miners]", minersHtml(snap), root);
  }

  function buildAdaptiveShell(root) {
    root.innerHTML =
      '<div class="tn-card">' +
      "<h3>What AI is for</h3>" +
      '<p class="tn-lead">AI does <b>not</b> find blocks and does <b>not</b> move the 45/45/10 split. The seed hands out small, rematched jobs so the network can practice and keep one shared model. If a result does not rematch, it does not count.</p>' +
      '<div class="tn-job"><h4>Immune exam</h4><p><b>Trying to improve:</b> honesty of the GPU lane. Every Fusion template names one protocol sim.</p><p><b>Cannot change:</b> who wins the block, or how the 50 MESH is split.</p></div>' +
      '<div class="tn-job"><h4>Protocol sim · cpu</h4><p><b>Trying to improve:</b> a picture of chain health (spread, spam, backlog). May nudge <i>practice intensity</i> only.</p><p><b>Cannot change:</b> block time, emission, or consensus difficulty.</p></div>' +
      '<div class="tn-job"><h4>Shared brain train · cuda</h4><p><b>Trying to improve:</b> one network MNIST model (v2). Your 3090 trains; the seed rematches the step.</p><p><b>Cannot change:</b> market units after height 1. Epoch going up is the win — not extra MESH.</p></div>' +
      "</div>" +
      aiPanelHtml() +
      '<div class="tn-card"><h3>What research is poking right now</h3>' +
      '<p class="tn-card__lead" data-ai-focus>Optional needles — not a claim that we are post-quantum or that AI runs the chain.</p>' +
      '<div data-ai-focus-rows></div></div>' +
      '<div class="tn-card tn-card--stories"><h3>Soft knobs only</h3>' +
      '<p class="tn-card__lead">Practice intensity and a mining <i>hint</i>. BPS, opcodes, and the tip stay locked.</p>' +
      '<div class="tn-soft-stories" data-soft-stories></div>' +
      '<p class="tn-muted tn-soft-why" data-rationale></p></div>' +
      '<div class="tn-grid">' +
      '<div class="tn-card"><h3>Consensus schedule</h3>' +
      rowSlot("epoch", "Settings version") +
      rowSlot("cdiff", "Consensus difficulty") +
      rowSlot("sdiff", "Soft difficulty hint") +
      rowSlot("rtint", "Retarget interval") +
      rowSlot("rtstep", "Retarget step") +
      rowSlot("rtfloor", "Min difficulty floor") +
      "</div>" +
      '<div class="tn-card"><h3>Practice knobs</h3>' +
      rowSlot("thresh", "Extra-practice trigger") +
      rowSlot("rounds", "Benchmark rounds") +
      rowSlot("minv", "Verifier floor") +
      rowSlot("bias", "CPU hint bias") +
      rowSlot("stipend", "Idle stipend cap") +
      "</div></div>";
  }

  function buildHowShell(root) {
    root.innerHTML =
      '<div class="tn-card"><h3>How a block is found</h3>' +
      '<p class="tn-lead">One digest. Two lanes. Both required. A warehouse of only CPUs or only GPUs is weaker than a normal gaming PC.</p>' +
      '<div class="tn-steps">' +
      '<div class="tn-step"><small>1</small><strong>GPU work</strong><span>Bandwidth-hard Fusion wave on the mixed pad. This ticket must exist first.</span></div>' +
      '<div class="tn-step"><small>2</small><strong>CPU work</strong><span>Latency-hard seal bound to that GPU ticket. Cannot run first (pow v5).</span></div>' +
      '<div class="tn-step"><small>3</small><strong>Fuse</strong><span>One digest. First valid nonce is the block. Official miners refuse CPU-only.</span></div>' +
      '<div class="tn-step"><small>Pay</small><strong>90 / 10</strong><span>Finder 90% (one Fusion pay). Nodes 10%. From height 50000.</span></div>' +
      "</div></div>" +
      '<div class="tn-card"><h3>MeshHash-Fusion (v5 sequential from 29,000)</h3>' +
      '<p class="tn-lead">GPU does one job. CPU does the other. They fuse into one digest. Not RandomX (CPU-only) and not KawPow (GPU-only).</p>' +
      '<ol class="tn-algo-steps">' +
      "<li><b>Work seed.</b> From the template: header commitment, the period recipe (pad size / mix rounds / fold salt), and the previous block hash.</li>" +
      "<li><b>Fill the pad.</b> Blake3 expands that seed into a 16, 32, or 64&nbsp;MiB scratchpad. This is host-side — that is why CPU H/s often reads higher than GPU H/s.</li>" +
      "<li><b>Mix.</b> Forward (and reverse) data-dependent reads/writes over the pad. CPU does this in DRAM. GPU does the same mix in VRAM so a 3090 can keep up.</li>" +
      "<li><b>GPU work.</b> 32 wavefronts × 64 gathers over that pad. This ticket is required before the CPU seal (v5 from height 29,000).</li>" +
      "<li><b>CPU work.</b> Latency-hard seal hashed with the GPU ticket. It cannot be computed first.</li>" +
      "<li><b>Fuse.</b> One digest. If either job is missing or faked, the seed rejects the block.</li>" +
      "<li><b>Difficulty.</b> First nonce whose digest meets consensus difficulty wins. Target spacing ~5&nbsp;s. AI is not in this path.</li>" +
      "</ol>" +
      '<pre class="tn-algo" aria-label="Fusion digest formula">work_seed = H(header_commitment || period_recipe || prev_hash)\n' +
      "gpu_wave  = Fusion wavefront (32 × 64)          // GPU first\n" +
      'cpu_fold  = Blake3(pad || salt || gpu_wave || "cpu-v5")\n' +
      'digest    = H(cpu_fold || gpu_wave || salt || pad_len || "v5")</pre>' +
      '<p class="tn-muted">The network rematches the winner with the same formula. Optional AI exams do not produce the digest.</p></div>' +
      '<div class="tn-card"><h3>Why an attacker cannot skip a lane</h3>' +
      '<p class="tn-lead">Every full node recomputes both lanes from the header. The miner does not submit a CPU hash and a GPU hash separately — only a nonce. If either lane is missing or taken from another block, the digest is wrong and the block is rejected.</p>' +
      '<div class="tn-row"><span>GPU-only farm</span><span>Still needs the CPU fold on that same pad</span></div>' +
      '<div class="tn-row"><span>CPU-only farm</span><span>Still needs the GPU wave on that same pad</span></div>' +
      '<div class="tn-row"><span>Replay</span><span>Work seed includes the previous block hash</span></div>' +
      '<div class="tn-row"><span>Precompute</span><span>Next pad is unknown until the last block exists</span></div>' +
      '<div class="tn-row"><span>Difficulty</span><span>One target on the bound digest — not two</span></div>' +
      '<div class="tn-row"><span>51%</span><span>Need a majority of Fusion work (both lanes), not GPU rental alone</span></div>' +
      '<p class="tn-muted" style="margin-top:.65rem">A colluding CPU shop and GPU shop can still share a 16–64&nbsp;MiB pad. Fusion makes a one-sided warehouse weaker than a home PC. It does not ban split-shop mining.</p></div>' +
      '<div class="tn-card"><h3>Why pay is 90 / 10</h3>' +
      '<p class="tn-lead">Fusion is one digest, so there is one finder pot — not two miner markets. From height 50000 that pot is 90%. Nodes keep 10% for attested work. Hashrate and AI cannot vote this away.</p>' +
      '<div class="tn-row"><span>Finder</span><span>90% · CPU + GPU, one hash</span></div>' +
      '<div class="tn-row"><span>Network nodes</span><span>10% · 5 MESH</span></div>' +
      '<div class="tn-row"><span>AI exams</span><span>Optional checks · do not find the block</span></div>' +
      '<div class="tn-row"><span>One digest</span><span>Both chips required or the block is rejected</span></div>' +
      '<p class="tn-muted" style="margin-top:.65rem">A GPU farm still needs the CPU seal. A CPU botnet still needs the GPU wave. That is how ordinary home PCs stay in the race.</p></div>' +
      '<div class="tn-grid">' +
      '<div class="tn-card"><h3>Why this instead of RandomX / KawPow</h3>' +
      '<div class="tn-row"><span>CPU-only (RandomX-like)</span><span>GPUs sit idle · botnets win</span></div>' +
      '<div class="tn-row"><span>GPU-only (KawPow-like)</span><span>Home CPUs sit idle · rental farms win</span></div>' +
      '<div class="tn-row"><span>Two separate PoWs</span><span>Attackers ignore the weaker market</span></div>' +
      '<div class="tn-row"><span>Fusion (MESH)</span><span>One digest, both lanes required</span></div>' +
      "</div>" +
      '<div class="tn-card"><h3>What miners do</h3>' +
      "<p class=\"tn-muted\">Point MonkeyMesh Miner at <span class=\"tn-mono\">https://eu.hashmonkeys.cloud</span>. Tick CPU and a GPU. Paste your <span class=\"tn-mono\">mesh01…</span> address.</p>" +
      "<p class=\"tn-muted\">Best rig is both on one machine. GPU POW pausing during a CUDA brain train is the same card taking a turn — not a stalled chain.</p></div>" +
      "</div>" +
      '<div class="tn-card"><h3>What AI does not do</h3>' +
      "<p class=\"tn-muted\">It does not find the block. It does not move emission. It does not upgrade the chain by itself. Protocol sims and the shared brain are rematched homework sitting beside Fusion mining. This is not a GPU rental marketplace.</p></div>";
  }

  function buildTokenomicsShell(root) {
    root.innerHTML =
      '<div class="tn-card"><h3>Tokenomics (live testnet)</h3>' +
      '<p class="tn-lead">Simple on purpose. Finder 90% / nodes 10% from height 50000. Hashrate and AI cannot vote it away.</p>' +
      rowSlot("tick", "Ticker") +
      rowSlot("cap", "Max supply") +
      rowSlot("emitted", "Total emitted") +
      rowSlot("remain", "Remaining") +
      rowSlot("issued", "Issued") +
      rowSlot("btime", "Block time") +
      rowSlot("breward", "Block reward") +
      rowSlot("cpu", "Finder") +
      rowSlot("gpu", "GPU lane (until 50000)") +
      rowSlot("node", "Network nodes") +
      rowSlot("mature", "Coinbase maturity") +
      rowSlot("pow", "PoW") +
      rowSlot("addr", "Addresses") +
      '<p class="tn-muted" style="margin-top:.75rem">No MESH dev fee in the coinbase. Finder gets 45. Nodes get 5 only for attested useful work (relay, routing, snapshot). Idle reputation is 0. Optional AI does not change this.</p></div>' +
      '<div class="tn-card"><h3>Supply by era (4-year halvings)</h3>' +
      '<div class="tn-row"><span>Years</span><span>Reward / block · added</span></div>' +
      '<div class="tn-row"><span>0–4</span><span>50 MESH · 1,261,440,000</span></div>' +
      '<div class="tn-row"><span>4–8</span><span>25 MESH · 630,720,000</span></div>' +
      '<div class="tn-row"><span>8–12</span><span>12.5 MESH · 315,360,000</span></div>' +
      '<div class="tn-row"><span>12–16</span><span>6.25 MESH · 157,680,000</span></div>' +
      '<div class="tn-row"><span>16+</span><span>Halves every 4 years → 2,522,880,000</span></div>' +
      '<p class="tn-muted" style="margin-top:.75rem">Cap is 50 × 25,228,800 × 2. Consensus clamps the subsidy so issuance cannot pass it.</p></div>' +
      '<div class="tn-card"><h3>Live markets</h3>' +
      rowSlot("height", "Height") +
      rowSlot("finality", "Finality") +
      rowSlot("split", "Fair split") +
      rowSlot("cpuLive", "Fusion seal now") +
      rowSlot("gpuLive", "GPU work now") +
      rowSlot("nodeLive", "Nodes now") +
      "</div>";
  }

  function updateTokenomics(root, snap) {
    var info = snap.info || {};
    var markets = snap.markets || {};
    setText('[data-row="tick"] [data-v]', "MESH", root);
    setText('[data-row="cap"] [data-v]', "2,522,880,000 MESH", root);
    var supply = supplyFromSnap(snap);
    setText('[data-row="emitted"] [data-v]', fmtMeshInt(supply.emitted) || "—", root);
    setText('[data-row="remain"] [data-v]', fmtMeshInt(supply.remain) || "—", root);
    setText('[data-row="issued"] [data-v]', supply.pct.toFixed(4) + "% of cap", root);
    setText('[data-row="btime"] [data-v]', "~5 seconds", root);
    setText('[data-row="breward"] [data-v]', markets.block_reward || "50 MESH", root);
    setText('[data-row="cpu"] [data-v]', "45% · Fusion seal", root);
    setText('[data-row="gpu"] [data-v]', "45% · GPU work", root);
    setText('[data-row="node"] [data-v]', "10% · 5 MESH", root);
    setText('[data-row="mature"] [data-v]', "20 confirmations", root);
    setText('[data-row="pow"] [data-v]', "Fusion v5 sequential from 29,000", root);
    setText('[data-row="addr"] [data-v]', "Ed25519 · mesh01…", root);
    setText('[data-row="height"] [data-v]', info.height ?? "—", root);
    setText('[data-row="finality"] [data-v]', finalityLabel(info), root);
    setText(
      '[data-row="split"] [data-v]',
      markets.finder_unify
        ? "90 / 10"
        : markets.fair_split
        ? "45 / 45 / 10 until #50000"
        : "see markets",
      root
    );
    setText('[data-row="cpuLive"] [data-v]', markets.cpu_market || "—", root);
    setText('[data-row="gpuLive"] [data-v]', markets.gpu_market || "—", root);
    setText('[data-row="nodeLive"] [data-v]', markets.node_market || "—", root);
  }

  function buildRoadmapShell(root) {
    root.innerHTML =
      '<div class="tn-card tn-road-hero">' +
      "<h3>Roadmap</h3>" +
      '<p class="tn-lead">MESH is a public testnet today. The path is simple: prove the chain, launch mainnet, then grow miners, nodes, and markets.</p>' +
      '<ol class="tn-road-strip" aria-label="Roadmap phases">' +
      '<li class="is-done"><b>1</b><span>Foundation</span></li>' +
      '<li class="is-now"><b>2</b><span>Public testnet</span></li>' +
      "<li><b>3</b><span>Harden</span></li>" +
      "<li><b>4</b><span>Mainnet</span></li>" +
      "<li><b>5</b><span>Listings</span></li>" +
      "<li><b>6</b><span>Growth</span></li>" +
      "</ol></div>" +
      '<div class="tn-road">' +
      '<article class="tn-road__item is-done"><div class="tn-road__mark" aria-hidden="true">1</div><div><span class="tn-kicker">Shipped</span><h4>Foundation</h4><p>Home-PC Fusion mining. Finder 90% / nodes 10% from height 50000. Desktop miner, wallet, HTTPS pool, and this explorer. Optional AI exams do not find blocks.</p><ul class="tn-road__ticks"><li>CPU + GPU find one block</li><li>45 MESH to the miner, 5 to nodes</li><li>Public testnet explorer</li></ul></div></article>' +
      '<article class="tn-road__item is-now"><div class="tn-road__mark" aria-hidden="true">2</div><div><span class="tn-kicker">Now</span><h4>Public testnet</h4><p>The live network is open. Miners earn MESH, wallets mature coinbase after 20 confirms, and the public pages stay in sync with the chain. This is the proving ground — not a tradable mainnet yet.</p><ul class="tn-road__ticks"><li>Open mining and node rewards</li><li>Docs a new visitor can actually read</li><li>Steady blocks, honest stats</li></ul></div></article>' +
      '<article class="tn-road__item"><div class="tn-road__mark" aria-hidden="true">3</div><div><span class="tn-kicker">Next</span><h4>Harden the network</h4><p>More independent public nodes, stronger finality, and an external security review. A long stability run with frozen rules before anyone should treat balances as money.</p><ul class="tn-road__ticks"><li>Multi-region public nodes</li><li>Independent PoW and consensus review</li><li>Signed releases and a bug bounty</li></ul></div></article>' +
      '<article class="tn-road__item"><div class="tn-road__mark" aria-hidden="true">4</div><div><span class="tn-kicker">Planned</span><h4>Mainnet launch</h4><p>A public genesis, production wallets and nodes, and a frozen emission schedule. Mainnet starts only after the harden phase is actually done.</p><ul class="tn-road__ticks"><li>Genesis ceremony</li><li>Production miner, node, and wallet</li><li>Clear emission and maturity rules</li></ul></div></article>' +
      '<article class="tn-road__item"><div class="tn-road__mark" aria-hidden="true">5</div><div><span class="tn-kicker">Planned</span><h4>Markets and listings</h4><p>After mainnet is live and stable: DEX pairs, CEX applications, and market-data pages so MESH can be found, priced, and traded like a normal coin.</p><ul class="tn-road__ticks"><li>Decentralized exchange pairs</li><li>Centralized exchange applications</li><li>Coin tracker and market-data listings</li></ul></div></article>' +
      '<article class="tn-road__item"><div class="tn-road__mark" aria-hidden="true">6</div><div><span class="tn-kicker">Planned</span><h4>Grow the network</h4><p>More pools and regions, easier onboarding, and a larger miner and node community. Partnerships and tooling that help people mine, hold, and run MESH — not a compute rental marketplace.</p><ul class="tn-road__ticks"><li>More public pools and regions</li><li>Mining community and guides</li><li>Wallets, explorers, and integrations</li></ul></div></article>' +
      "</div>" +
      '<p class="tn-muted tn-road-foot">MESH is a home-PC coin with optional rematched AI exams. It is not a GPU rental marketplace, and AI cannot change the 90 / 10 split.</p>';
  }

  function buildMarketShell(root) {
    root.innerHTML =
      '<div class="tn-card">' +
      "<h3>Homework board</h3>" +
      '<p class="tn-lead">Optional homework. GPU miners can post AI jobs; any CPU can rematch them. The seed re-runs the job. This does not find the block and does not move 90 / 10. Fusion pads stay on one PC.</p>' +
      "</div>" +
      '<div class="tn-grid">' +
      '<div class="tn-card"><h3>Shared brain</h3>' +
      rowSlot("mepoch", "Brain epoch") +
      rowSlot("madv", "Advances") +
      rowSlot("macc", "Last accuracy") +
      '<p class="tn-muted" style="margin-top:.5rem">The seed steps the model when no GPU worker is attached so the epoch cannot sit at 0.</p></div>' +
      '<div class="tn-card"><h3>This block\'s homework pot</h3>' +
      rowSlot("mexam", "Exam / brain floor") +
      rowSlot("mfusion", "Finder GPU work") +
      rowSlot("mneed", "Exam required") +
      "</div></div>" +
      '<div class="tn-card"><h3>Pending CPU seals</h3><div data-seal-pending></div>' +
      '<p class="tn-muted" style="margin-top:.5rem">Last MATCH: <span data-seal-last>—</span></p></div>' +
      '<div class="tn-card"><h3>Recent exam MATCH</h3><div data-exam-tape></div></div>';
  }

  function updateMarket(root, snap) {
    var pulse = snap.pulse || {};
    var markets = snap.markets || {};
    setText('[data-row="mepoch"] [data-v]', pulse.brain_epoch != null ? pulse.brain_epoch : "—", root);
    setText('[data-row="madv"] [data-v]', pulse.brain_advances != null ? pulse.brain_advances : "—", root);
    var acc = pulse.brain_acc;
    setText(
      '[data-row="macc"] [data-v]',
      acc != null && acc !== "" ? (Number(acc) * 100).toFixed(1) + "%" : "—",
      root
    );
    setText(
      '[data-row="mexam"] [data-v]',
      markets.finder_unify
        ? "off — optional"
        : markets.gpu_exam_market || (markets.helper_floor ? "on" : "off"),
      root
    );
    setText(
      '[data-row="mfusion"] [data-v]',
      markets.gpu_fusion_market || markets.gpu_market || "—",
      root
    );
    setText(
      '[data-row="mneed"] [data-v]',
      markets.finder_unify ? "no" : markets.exam_required ? "yes until #50000" : "no",
      root
    );
    var seals = snap.seals || {};
    var pend = seals.pending || [];
    var box = root.querySelector("[data-seal-pending]");
    if (box) {
      if (!pend.length) {
        box.innerHTML = '<p class="tn-muted">No GPU offers waiting for a CPU seal.</p>';
      } else {
        box.innerHTML = pend
          .slice(0, 8)
          .map(function (o) {
            return (
              '<div class="tn-row"><span class="tn-mono">' +
              (o.kind || "job") +
              '</span><span class="tn-mono">' +
              String(o.job_id || "").slice(0, 22) +
              "</span></div>"
            );
          })
          .join("");
      }
    }
    var last = seals.last_match || {};
    var lastEl = root.querySelector("[data-seal-last]");
    if (lastEl) {
      lastEl.textContent = last.job_id
        ? (last.kind || "job") + " · sealer " + String(last.sealer || "").slice(0, 14)
        : "—";
    }
    updateAiPanel(root, snap);
  }

  function updateAdaptive(root, snap) {
    updateAiPanel(root, snap);
    var env = snap.envelopes || {};
    var soft = env.envelopes || {};
    var retarget = env.retarget || {};
    var gate = env.quantum_gate || (snap.quantum && snap.quantum.self_evolution) || {};
    var latest = env.latest_epoch;
    setText('[data-row="epoch"] [data-v]', env.param_epoch ?? "—", root);
    setText('[data-row="cdiff"] [data-v]', env.consensus_difficulty ?? snap.info.next_difficulty ?? "—", root);
    setText('[data-row="sdiff"] [data-v]', env.soft_diff_hint ?? snap.info.soft_diff_hint ?? "—", root);
    setText(
      '[data-row="rtint"] [data-v]',
      retarget.interval != null ? retarget.interval : soft.retarget_interval ?? "—",
      root
    );
    setText(
      '[data-row="rtstep"] [data-v]',
      retarget.step != null ? retarget.step : soft.retarget_step ?? "—",
      root
    );
    setText(
      '[data-row="rtfloor"] [data-v]',
      retarget.min_floor != null ? retarget.min_floor : soft.min_difficulty_floor ?? "—",
      root
    );
    var gSince = Number(gate.grover_certs_since_retarget_adapt);
    var gNeed = Number(gate.min_grover_certs_for_retarget || 5);
    setText(
      '[data-row="qgate"] [data-v]',
      isFinite(gSince) ? Math.min(gSince, gNeed) + " / " + gNeed : "—",
      root
    );
    var why =
      latest && latest.rationale
        ? humanizeRationale(latest.rationale)
        : "Waiting for checked AI findings (soft knobs) and/or Grover certificates (retarget schedule).";
    setText("[data-rationale]", why, root);
    var board = trilemmaBoard(snap);
    var q = (snap.quantum && snap.quantum.board) || {};
    var focus =
      "Trilemma weakest: " +
      (board.weakest || "—") +
      (board.decent != null ? " (decent " + board.decent + "/100)" : "") +
      ". Quantum readiness " +
      (q.readiness != null ? q.readiness : "—") +
      "/100 — weakest " +
      (q.weakest || "—") +
      ". These needles do not move the 45/45/10 split.";
    setText("[data-ai-focus]", focus, root);
    var br = liveBrain(snap);
    var focusRows =
      rowSlot("brv", "Shared brain") +
      rowSlot("qready", "Quantum readiness") +
      rowSlot("weak", "Weakest research leg");
    var slot = root.querySelector("[data-ai-focus-rows]");
    if (slot && !slot.dataset.ready) {
      slot.innerHTML = focusRows;
      slot.dataset.ready = "1";
    }
    setText('[data-row="brv"] [data-v]', br.epoch > 0 ? "v" + br.ver + " · epoch " + br.epoch : "idle", root);
    setText('[data-row="qready"] [data-v]', q.readiness != null ? q.readiness + " / 100" : "—", root);
    setText('[data-row="weak"] [data-v]', (board.weakest || "—") + " / " + (q.weakest || "—"), root);
    setText('[data-row="thresh"] [data-v]', soft.soft_adapt_signal_threshold ?? "—", root);
    setText('[data-row="rounds"] [data-v]', soft.soft_benchmark_rounds ?? "—", root);
    setText('[data-row="minv"] [data-v]', soft.min_verifier_weight ?? "—", root);
    setText('[data-row="bias"] [data-v]', soft.suggested_cpu_diff_bias ?? "—", root);
    setText('[data-row="stipend"] [data-v]', soft.idle_stipend_bps_cap ?? "—", root);

    var stories = softStories(snap);
    var html = stories
      .map(function (s) {
        return (
          '<article class="tn-soft-story tn-soft-story--' +
          esc(s.tone || "neutral") +
          '"><h4>' +
          esc(s.title) +
          "</h4><p>" +
          esc(s.body) +
          "</p></article>"
        );
      })
      .join("");
    setHTML("[data-soft-stories]", html, root);
  }

  function buildBlocksShell(root) {
    root.innerHTML =
      '<div class="tn-card"><h3>Recent blocks</h3>' +
      '<p class="tn-card__lead">Newest first · click a row for detail</p>' +
      '<div class="tn-blocks-head" aria-hidden="true"><span>Block</span><span>Detail</span><span>Time</span><span>Hash</span></div>' +
      '<div class="tn-blocks" data-block-list></div></div>';
  }

  function syncBlockList(root, blocks) {
    var list = root.querySelector("[data-block-list]");
    if (!list) return;
    var want = {};
    blocks.forEach(function (b) {
      want[String(b.height)] = b;
    });
    Array.prototype.slice.call(list.children).forEach(function (el) {
      if (!want[el.dataset.height]) el.remove();
    });
    blocks.forEach(function (b) {
      var key = String(b.height);
      var el = list.querySelector('[data-height="' + key + '"]');
      var payout =
        b.txs && b.txs[0] && b.txs[0].outputs && b.txs[0].outputs[0]
          ? b.txs[0].outputs[0].address
          : "";
      if (!el) {
        el = document.createElement("a");
        el.className = "tn-block-row";
        el.dataset.height = key;
        el.innerHTML =
          '<div class="tn-block-row__index" data-idx></div>' +
          '<div class="tn-block-row__main"><div class="tn-block-row__learn" data-detail></div></div>' +
          '<div class="tn-block-row__time" data-time></div>' +
          '<div class="tn-block-row__hash" data-hash></div>';
      }
      el.href = "#block/" + key;
      setText("[data-idx]", "#" + b.height, el);
      setText(
        "[data-detail]",
        "diff " +
          b.difficulty +
          " · " +
          ((b.txs && b.txs.length) || 0) +
          " txs" +
          (payout ? " · paid " + shortAddr(payout) : ""),
        el
      );
      setText("[data-time]", fmtTime(b.timestamp), el);
      setText("[data-hash]", shortHash(b.id), el);
      list.appendChild(el);
    });
  }

  function buildNetworkShell(root) {
    root.innerHTML =
      '<div class="tn-card"><h3>Public seed</h3>' +
      rowSlot("nnodes", "Active nodes") +
      rowSlot("peer", "Peer id", true) +
      rowSlot("rpc", "Connect (RPC)", true) +
      rowSlot("pool", "Pool mine target", true) +
      rowSlot("p2p", "Connect (P2P)", true) +
      rowSlot("nheight", "Height") +
      rowSlot("nfinality", "Finality") +
      rowSlot("genesis", "Genesis", true) +
      '</div><div class="tn-card"><h3>Nodes the seed sees</h3><div data-nodes></div></div>' +
      '<div class="tn-card"><h3>Known miners</h3><div data-miners></div></div>';
  }

  function updateNetwork(root, snap) {
    var info = snap.info || {};
    setText('[data-row="nnodes"] [data-v]', liveNodeCount(info), root);
    setText('[data-row="peer"] [data-v]', shortHash(info.peer_id), root);
    setText('[data-row="rpc"] [data-v]', "seednode.hashmonkeys.cloud:18080", root);
    setText('[data-row="pool"] [data-v]', "https://eu.hashmonkeys.cloud", root);
    setText('[data-row="p2p"] [data-v]', "seednode.hashmonkeys.cloud:39001", root);
    setText('[data-row="nheight"] [data-v]', info.height ?? "—", root);
    setText('[data-row="nfinality"] [data-v]', finalityLabel(info), root);
    setText('[data-row="genesis"] [data-v]', shortHash(info.genesis), root);
    setHTML("[data-nodes]", nodePeerListHtml(info), root);
    setHTML("[data-miners]", minersHtml(snap), root);
  }

  async function renderDetailBlock(root, height) {
    var key = "block:" + height;
    if (state.builtPage !== key) {
      root.innerHTML = '<div class="tn-card"><p class="tn-muted">Loading block…</p></div>';
      state.builtPage = key;
    }
    var b = await api("/v1/getblock?height=" + encodeURIComponent(height));
    var outs = (b.txs || [])
      .map(function (tx) {
        var pe = pomcExplain(tx.memo);
        var n = (tx.outputs || []).length;
        var total = sumTxOut(tx);
        var lines = (tx.outputs || [])
          .map(function (o, idx) {
            return (
              '<div class="tn-out-line"><span class="' +
              lanePillClass(outRole(idx, n, tx.memo, o)) +
              '">' +
              esc(outRole(idx, n, tx.memo, o)) +
              "</span> <strong>" +
              esc(o.amount) +
              "</strong> → " +
              addrLink(o.address) +
              (o.paid_for
                ? '<div class="tn-muted" style="margin-top:.15rem">' +
                  esc(o.paid_for) +
                  "</div>"
                : "") +
              "</div>"
            );
          })
          .join("");
        return (
          '<div class="tn-tx-card">' +
          '<div class="tn-tx-card__head"><a class="tn-mono" href="#tx/' +
          esc(tx.txid) +
          '">' +
          esc(shortHash(tx.txid)) +
          "</a>" +
          (pe
            ? '<span class="tn-pill">' + esc(pe.kind) + "</span>"
            : '<span class="tn-pill tn-pill--muted">tx</span>') +
          "</div>" +
          '<p class="tn-muted">' +
          esc(pe ? pe.detail : tx.memo || "Transaction") +
          (total ? " · total " + esc(fmtAtomic(total)) : "") +
          "</p>" +
          lines +
          "</div>"
        );
      })
      .join("");
    root.innerHTML =
      '<div class="tn-card"><h3>Block #' +
      esc(b.height) +
      "</h3>" +
      rowSlot("id", "Hash", true).replace(">—", ">" + esc(b.id)) +
      rowSlot("prev", "Previous", true).replace(">—", ">" + esc(b.prev)) +
      rowSlot("time", "Time").replace(">—", ">" + esc(fmtTime(b.timestamp))) +
      rowSlot("diff", "Difficulty").replace(">—", ">" + esc(b.difficulty)) +
      rowSlot("nonce", "Nonce").replace(">—", ">" + esc(b.nonce)) +
      rowSlot("txc", "Transactions").replace(">—", ">" + esc((b.txs && b.txs.length) || 0)) +
      "</div>" +
      '<div class="tn-card"><h3>Coin movements</h3>' +
      (outs || '<p class="tn-muted">None</p>') +
      '</div><p class="tn-back"><a href="#blocks">← Back to blocks</a></p>';
  }

  async function renderTxs(root, snap) {
    if (state.builtPage !== "txs") {
      root.innerHTML =
        '<div class="tn-card"><h3>Recent transactions</h3>' +
        '<p class="tn-card__lead">Mempool + recent blocks · click for full outputs / UTXO links</p>' +
        '<div class="tn-blocks-head" aria-hidden="true"><span>Where</span><span>Detail</span><span>Total</span><span>Tx</span></div>' +
        '<div class="tn-blocks" data-tx-list></div></div>';
      state.builtPage = "txs";
    }
    var height = snap.info && snap.info.height != null ? snap.info.height : 0;
    var pair = await Promise.all([
      fetchRecentBlocks(height, 20),
      api("/v1/mempool").catch(function () { return { txs: [] }; }),
    ]);
    var rows = [];
    (pair[1].txs || []).forEach(function (tx) {
      rows.push({ tx: tx, height: null });
    });
    pair[0].forEach(function (b) {
      (b.txs || []).forEach(function (tx) {
        rows.push({ tx: tx, height: b.height });
      });
    });
    rows = rows.slice(0, 50);
    var list = root.querySelector("[data-tx-list]");
    if (!list) return;
    list.innerHTML = rows
      .map(function (r) {
        var tx = r.tx;
        var pe = pomcExplain(tx.memo);
        var n = (tx.outputs || []).length;
        var total = sumTxOut(tx);
        var first = (tx.outputs && tx.outputs[0]) || {};
        return (
          '<a class="tn-block-row" href="#tx/' +
          esc(tx.txid) +
          '"><div class="tn-block-row__index">' +
          (r.height != null ? "#" + esc(r.height) : "pool") +
          '</div><div class="tn-block-row__main"><div class="tn-block-row__learn">' +
          esc(pe ? pe.kind : tx.memo || "tx") +
          " · " +
          n +
          " out" +
          (first.address ? " · " + esc(shortAddr(first.address)) : "") +
          '</div></div><div class="tn-block-row__time">' +
          esc(total ? fmtAtomic(total) : first.amount || "—") +
          '</div><div class="tn-block-row__hash">' +
          esc(shortHash(tx.txid)) +
          "</div></a>"
        );
      })
      .join("") || '<p class="tn-muted">No recent transactions.</p>';
  }

  async function renderTx(root, txid, snap) {
    root.innerHTML = '<div class="tn-card"><p class="tn-muted">Loading…</p></div>';
    state.builtPage = "tx:" + txid;
    var height = snap.info && snap.info.height != null ? snap.info.height : 0;
    var blocks = await fetchRecentBlocks(height, 60);
    var found = null;
    var at = null;
    for (var i = 0; i < blocks.length; i++) {
      var hit = (blocks[i].txs || []).find(function (t) { return t.txid === txid; });
      if (hit) {
        found = hit;
        at = blocks[i].height;
        break;
      }
    }
    if (!found) {
      var mem = await api("/v1/mempool").catch(function () { return { txs: [] }; });
      found = (mem.txs || []).find(function (t) { return t.txid === txid; });
    }
    if (!found) {
      root.innerHTML =
        '<div class="tn-card"><h3>Not found</h3><p class="tn-muted">Not in recent window or mempool.</p></div>' +
        '<p class="tn-back"><a href="#txs">← Back</a></p>';
      return;
    }
    var pe = pomcExplain(found.memo);
    var outs = found.outputs || [];
    var total = sumTxOut(found);
    var outHtml = outs
      .map(function (o, idx) {
        return (
          '<div class="tn-utxo-row">' +
          '<div class="tn-utxo-row__meta"><span class="' +
          lanePillClass(outRole(idx, outs.length, found.memo, o)) +
          '">' +
          esc(outRole(idx, outs.length, found.memo, o)) +
          '</span><span class="tn-muted">vout ' +
          idx +
          "</span></div>" +
          '<div class="tn-utxo-row__amt">' +
          esc(o.amount) +
          "</div>" +
          '<div class="tn-muted">' +
          esc(o.paid_for || "") +
          "</div>" +
          '<div class="tn-utxo-row__addr">' +
          addrLink(o.address) +
          '<div class="tn-mono tn-addr-full">' +
          esc(o.address) +
          "</div></div></div>"
        );
      })
      .join("");
    root.innerHTML =
      '<div class="tn-card"><h3>' +
      esc(pe ? pe.kind : "Transaction") +
      "</h3>" +
      (pe ? '<p class="tn-card__lead">' + esc(pe.detail) + "</p>" : "") +
      '<div class="tn-row"><span>Txid</span><span class="tn-mono">' +
      esc(found.txid) +
      "</span></div>" +
      '<div class="tn-row"><span>Block</span><span>' +
      (at != null
        ? '<a href="#block/' + esc(at) + '">#' + esc(at) + "</a>"
        : "mempool") +
      "</span></div>" +
      '<div class="tn-row"><span>Memo</span><span class="tn-mono">' +
      esc(found.memo || "—") +
      "</span></div>" +
      '<div class="tn-row"><span>Outputs</span><span>' +
      esc(outs.length) +
      (total ? " · " + esc(fmtAtomic(total)) : "") +
      "</span></div>" +
      "</div>" +
      '<div class="tn-card"><h3>Creates these UTXOs</h3>' +
      '<p class="tn-card__lead">Each output is an unspent coin until spent. Click an address to list its UTXOs.</p>' +
      (outHtml || '<p class="tn-muted">No outputs.</p>') +
      '</div><p class="tn-back"><a href="#txs">← Back</a> · <a href="#utxos">UTXO lookup</a></p>';
  }

  function bindUtxoForm(root) {
    var form = root.querySelector("[data-utxo-form]");
    if (!form || form.dataset.bound) return;
    form.dataset.bound = "1";
    form.addEventListener("submit", function (ev) {
      ev.preventDefault();
      var input = form.querySelector("[data-utxo-addr]");
      var addr = (input && input.value || "").trim();
      if (!addr) return;
      location.hash = "#addr/" + encodeURIComponent(addr);
    });
  }

  async function renderUtxos(root, snap, addrHint) {
    if (state.builtPage !== "utxos" && state.builtPage !== "utxos:" + (addrHint || "")) {
      root.innerHTML =
        '<div class="tn-card"><h3>Look up coins (UTXOs)</h3>' +
        '<p class="tn-card__lead">Paste a mesh address to see balance and every unspent output.</p>' +
        '<form class="tn-utxo-form" data-utxo-form>' +
        '<input class="tn-utxo-form__input tn-mono" data-utxo-addr type="text" spellcheck="false" placeholder="mesh01…" autocomplete="off" />' +
        '<button type="submit" class="tn-utxo-form__btn">Look up</button></form>' +
        '<div data-utxo-result></div></div>' +
        '<div class="tn-card"><h3>Sample unspent coins</h3>' +
        '<p class="tn-card__lead">Live slice from the UTXO set · height <span data-utxo-tip>—</span> · set size <span data-utxo-count>—</span></p>' +
        '<div class="tn-blocks-head" aria-hidden="true"><span>Amt</span><span>Address</span><span>Out</span><span>Tx</span></div>' +
        '<div class="tn-blocks" data-utxo-sample></div></div>';
      state.builtPage = "utxos" + (addrHint ? ":" + addrHint : "");
      bindUtxoForm(root);
    }
    bindUtxoForm(root);

    var input = root.querySelector("[data-utxo-addr]");
    if (input && addrHint && input.value !== addrHint) input.value = addrHint;

    var sample = await api("/v1/snapshot/utxos?limit=25").catch(function () {
      return { utxos: [], utxo_count: 0, height: null };
    });
    setText("[data-utxo-tip]", sample.height ?? (snap.info && snap.info.height) ?? "—", root);
    setText("[data-utxo-count]", sample.utxo_count ?? "—", root);
    var sampleList = root.querySelector("[data-utxo-sample]");
    if (sampleList) {
      sampleList.innerHTML =
        (sample.utxos || [])
          .map(function (u) {
            return (
              '<a class="tn-block-row" href="#addr/' +
              encodeURIComponent(u.address) +
              '"><div class="tn-block-row__index">' +
              esc(fmtAtomic(u.atomic)) +
              '</div><div class="tn-block-row__main"><div class="tn-block-row__learn tn-mono">' +
              esc(shortAddr(u.address)) +
              '</div></div><div class="tn-block-row__time">vout ' +
              esc(u.vout) +
              '</div><div class="tn-block-row__hash">' +
              esc(shortHash(u.txid)) +
              "</div></a>"
            );
          })
          .join("") || '<p class="tn-muted">No sample available.</p>';
    }

    var result = root.querySelector("[data-utxo-result]");
    if (!result) return;
    if (!addrHint) {
      result.innerHTML =
        '<p class="tn-muted" style="margin-top:.85rem">Enter an address above, or click a sample row / any address link on a transaction.</p>';
      return;
    }

    result.innerHTML = '<p class="tn-muted" style="margin-top:.85rem">Loading…</p>';
    var pair = await Promise.all([
      api("/v1/getbalance?address=" + encodeURIComponent(addrHint)).catch(function () {
        return null;
      }),
      api("/v1/utxos?address=" + encodeURIComponent(addrHint)).catch(function () {
        return [];
      }),
      api("/v1/getrewards?address=" + encodeURIComponent(addrHint)).catch(function () {
        return null;
      }),
    ]);
    var bal = pair[0];
    var list = Array.isArray(pair[1]) ? pair[1] : pair[1].utxos || [];
    var rewards = pair[2];
    if (!bal && !list.length) {
      result.innerHTML =
        '<p class="tn-muted" style="margin-top:.85rem">No balance found for <span class="tn-mono">' +
        esc(addrHint) +
        "</span>.</p>";
      return;
    }
    var rows = list
      .map(function (u) {
        var why = u.title
          ? esc(u.title) +
            (u.paid_for ? " — " + esc(u.paid_for) : "") +
            " · "
          : "";
        return (
          '<a class="tn-utxo-row tn-utxo-row--link" href="#tx/' +
          esc(u.txid) +
          '"><div class="tn-utxo-row__meta"><span class="' +
          lanePillClass(u.title) +
          '">' +
          esc(u.title || ("vout " + u.vout)) +
          '</span><span class="tn-mono">' +
          esc(shortHash(u.txid)) +
          "</span></div>" +
          '<div class="tn-utxo-row__amt">' +
          esc(u.amount || fmtAtomic(u.atomic)) +
          "</div>" +
          '<div class="tn-muted">' +
          why +
          (u.mature === false
            ? "Immature coinbase · " + esc(u.confirmations != null ? u.confirmations : "?") + "/20 conf"
            : "Unspent · click for creating tx") +
          "</div></a>"
        );
      })
      .join("");
    var laneHtml = "";
    if (rewards && rewards.by_lane && rewards.by_lane.length) {
      laneHtml =
        '<h4 class="tn-utxo-list-title">Paid for</h4>' +
        rewards.by_lane
          .map(function (l) {
            return (
              '<div class="tn-row"><span><span class="' +
              lanePillClass(l.title) +
              '">' +
              esc(l.title) +
              "</span> · " +
              esc(l.paid_for || "") +
              '</span><span><strong>' +
              esc(l.amount) +
              "</strong> · " +
              esc(l.count) +
              "</span></div>"
            );
          })
          .join("") +
        (rewards.recent && rewards.recent.length
          ? '<h4 class="tn-utxo-list-title">Recent coinbase</h4>' +
            rewards.recent
              .slice(0, 16)
              .map(function (h) {
                return (
                  '<div class="tn-row"><span>#' +
                  esc(h.height) +
                  " · " +
                  esc(h.title) +
                  (h.mature ? "" : " · immature") +
                  "</span><span>" +
                  esc(h.amount) +
                  "</span></div>"
                );
              })
              .join("")
          : "");
    }
    result.innerHTML =
      '<div class="tn-utxo-summary">' +
      '<div class="tn-row"><span>Address</span><span class="tn-mono">' +
      esc(addrHint) +
      "</span></div>" +
      '<div class="tn-row"><span>Balance</span><span><strong>' +
      esc((bal && bal.balance) || fmtAtomic(bal && bal.atomic) || "0") +
      "</strong></span></div>" +
      (bal && bal.spendable
        ? '<div class="tn-row"><span>Spendable</span><span>' +
          esc(bal.spendable) +
          "</span></div>"
        : "") +
      (bal && bal.immature && String(bal.immature_atomic || "0") !== "0"
        ? '<div class="tn-row"><span>Immature coinbase</span><span>' +
          esc(bal.immature) +
          " · 20 conf</span></div>"
        : "") +
      '<div class="tn-row"><span>Unspent outputs</span><span>' +
      esc(list.length) +
      "</span></div></div>" +
      laneHtml +
      '<h4 class="tn-utxo-list-title">UTXO list</h4>' +
      (rows || '<p class="tn-muted">No unspent outputs.</p>');
  }

  async function ensureShell(page) {
    var root = document.getElementById("tnRoot");
    if (!root) return root;
    var key = page.name + (page.id ? ":" + page.id : "");
    if (
      state.builtPage === key &&
      (page.name === "overview" ||
        page.name === "adaptive" ||
        page.name === "network" ||
        page.name === "blocks" ||
        page.name === "how" ||
        page.name === "tokenomics" ||
        page.name === "roadmap" ||
        page.name === "market")
    ) {
      return root;
    }
    if (page.name === "overview") {
      buildOverviewShell(root);
      state.builtPage = key;
    } else if (page.name === "adaptive") {
      buildAdaptiveShell(root);
      state.builtPage = key;
    } else if (page.name === "how") {
      buildHowShell(root);
      state.builtPage = key;
    } else if (page.name === "tokenomics") {
      buildTokenomicsShell(root);
      state.builtPage = key;
    } else if (page.name === "roadmap") {
      buildRoadmapShell(root);
      state.builtPage = key;
    } else if (page.name === "market") {
      buildMarketShell(root);
      state.builtPage = key;
    } else if (page.name === "blocks") {
      buildBlocksShell(root);
      state.builtPage = key;
    } else if (page.name === "network") {
      buildNetworkShell(root);
      state.builtPage = key;
    }
    return root;
  }

  var refreshing = false;

  async function refresh(forceShell) {
    if (refreshing) return;
    refreshing = true;
    var page = pageFromHash();
    setActiveTab(page.name);
    try {
      var snap = await fetchSnapshot();
      state.snap = snap;
      ensureHero(snap);
      var tip = mineTip(snap);
      var seedH = snap.info && snap.info.height;
      setStatus(
        seedBehindMine(snap)
          ? "Mine tip #" +
              tip +
              " · seed syncing #" +
              seedH +
              " · " +
              new Date().toLocaleTimeString()
          : "Chain height " + (tip || seedH || "—") + " · " + new Date().toLocaleTimeString(),
        true
      );

      var root = document.getElementById("tnRoot");
      if (!root) return;

      if (page.name === "overview") {
        await ensureShell(page);
        updateOverview(root, snap);
      } else if (page.name === "adaptive") {
        await ensureShell(page);
        updateAdaptive(root, snap);
      } else if (page.name === "how") {
        await ensureShell(page);
      } else if (page.name === "tokenomics") {
        await ensureShell(page);
        updateTokenomics(root, snap);
      } else if (page.name === "market") {
        await ensureShell(page);
        updateMarket(root, snap);
      } else if (page.name === "roadmap") {
        await ensureShell(page);
      } else if (page.name === "network") {
        await ensureShell(page);
        updateNetwork(root, snap);
      } else if (page.name === "blocks") {
        await ensureShell(page);
        var blocks = await fetchRecentBlocks(snap.info.height || 0, 40);
        state.blocks = blocks;
        syncBlockList(root, blocks);
      } else if (page.name === "block") {
        await renderDetailBlock(root, page.id);
      } else if (page.name === "txs") {
        await renderTxs(root, snap);
      } else if (page.name === "tx") {
        await renderTx(root, page.id, snap);
      } else if (page.name === "utxos") {
        await renderUtxos(root, snap, page.id);
      } else {
        page = { name: "overview", id: null };
        await ensureShell(page);
        updateOverview(root, snap);
      }
    } catch (err) {
      setStatus("Cannot reach MonkeyMesh seed — " + err.message, false);
      var root = document.getElementById("tnRoot");
      if (root && state.builtPage !== "offline") {
        root.innerHTML =
          '<div class="tn-card"><h3>Offline</h3><p class="tn-muted">The public seed is not answering right now. Try again in a moment.</p></div>';
        state.builtPage = "offline";
      }
    } finally {
      refreshing = false;
    }
  }

  window.addEventListener("hashchange", function () {
    state.builtPage = null;
    refresh(true);
  });

  refresh(true);
  setInterval(function () {
    var page = pageFromHash();
    // Soft poll: update numbers in place; never wipe overview/adaptive/network
    if (
      page.name === "overview" ||
      page.name === "adaptive" ||
      page.name === "network" ||
      page.name === "blocks" ||
      page.name === "tokenomics" ||
      page.name === "how" ||
      page.name === "roadmap" ||
      page.name === "market"
    ) {
      refresh(false);
    }
  }, 10000);
})();
