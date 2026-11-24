(() => {
  const FREQS = [60, 150, 400, 1000, 2400, 15000];
  const SDK = "https://sdk.scdn.co";

  if (window === window.top) {
    let current = null;
    const broadcast = () => {
      if (!current) return;
      for (const frame of document.querySelectorAll("iframe")) {
        try {
          frame.contentWindow.postMessage({ rustifyAudio: current }, SDK);
        } catch (e) {}
      }
    };
    window.addEventListener("message", e => {
      if (e.origin === SDK && e.data && e.data.rustifyAudioHello) broadcast();
    });
    window.rustifyAudio = {
      apply(settings) {
        current = settings;
        broadcast();
      },
    };
    return;
  }

  if (location.origin !== SDK) return;

  let settings = { eq: false, bands: [0, 0, 0, 0, 0, 0], level: "normal", mono: false, fade: 0, quality: "very_high", normalize: true };
  const gains = new Map();
  let trackGain = 0;

  const pick = list => {
    if (!Array.isArray(list)) return list;
    list.forEach(f => f && f.file_id && typeof f.gain_db === "number" && gains.set(f.file_id, f.gain_db));
    if (settings.quality !== "high") return list;
    const lower = list.filter(f => f && f.audio_quality !== "VERY_HIGH");
    return lower.length ? lower : list;
  };

  const rewrite = data => {
    for (const track of (data && data.state_machine && data.state_machine.tracks) || []) {
      const manifest = track && track.manifest;
      if (!manifest) continue;
      for (const key of Object.keys(manifest)) manifest[key] = pick(manifest[key]);
    }
    return data;
  };

  const rewriteText = text => {
    if (typeof text !== "string" || !text.includes("state_machine")) return text;
    try {
      const data = JSON.parse(text);
      for (const payload of data.payloads || []) rewrite(payload);
      if (data.state_machine) rewrite(data);
      return JSON.stringify(data);
    } catch (e) {
      return text;
    }
  };

  const WS = window.WebSocket;
  const wrapEvent = e => (typeof e.data === "string" && e.data.includes("state_machine") ? new MessageEvent("message", { data: rewriteText(e.data), origin: e.origin }) : e);
  class Socket extends WS {
    addEventListener(type, listener, options) {
      if (type !== "message" || typeof listener !== "function") return super.addEventListener(type, listener, options);
      return super.addEventListener(type, e => listener.call(this, wrapEvent(e)), options);
    }
    set onmessage(fn) {
      super.onmessage = typeof fn === "function" ? e => fn.call(this, wrapEvent(e)) : fn;
    }
    get onmessage() {
      return super.onmessage;
    }
  }
  window.WebSocket = Socket;

  const fetch0 = window.fetch;
  window.fetch = function (input, init) {
    const url = typeof input === "string" ? input : (input && input.url) || "";
    const audio = url.match(/\/audio\/([0-9a-f]{40})/);
    if (audio && gains.has(audio[1])) {
      trackGain = gains.get(audio[1]);
      graphs.forEach(tune);
    }
    const pending = fetch0.apply(this, arguments);
    if (!url.includes("/track-playback/v1/")) return pending;
    return pending.then(res => {
      if (!res.ok || res.status === 204) return res;
      return res.clone().text().then(text => {
        if (!text.includes("state_machine")) return res;
        try {
          return new Response(JSON.stringify(rewrite(JSON.parse(text))), { status: res.status, statusText: res.statusText, headers: res.headers });
        } catch (e) {
          return res;
        }
      }, () => res);
    });
  };
  const graphs = [];

  const levelGain = level => (level === "loud" ? 1.6 : level === "quiet" ? 0.6 : 1.0);

  const tune = g => {
    const now = g.ctx.currentTime;
    g.filters.forEach((f, i) => f.gain.setTargetAtTime(settings.eq ? Math.max(-12, Math.min(12, settings.bands[i] || 0)) : 0, now, 0.03));
    const norm = settings.normalize ? Math.pow(10, Math.max(-12, Math.min(12, trackGain)) / 20) : 1;
    g.gain.gain.setTargetAtTime(levelGain(settings.level) * norm, now, 0.05);
    g.limiter.threshold.setValueAtTime(settings.level === "loud" ? -3 : 0, now);
    g.limiter.ratio.setValueAtTime(settings.level === "loud" ? 12 : 1, now);
    g.mono.channelCount = settings.mono ? 1 : 2;
    g.mono.channelCountMode = settings.mono ? "explicit" : "max";
  };

  const wire = el => {
    if (el.__rustify) return;
    el.__rustify = true;
    try {
      const ctx = new AudioContext({ latencyHint: "playback" });
      const source = ctx.createMediaElementSource(el);
      const mono = ctx.createGain();
      mono.channelInterpretation = "speakers";
      const filters = FREQS.map((frequency, i) => {
        const f = ctx.createBiquadFilter();
        f.type = i === 0 ? "lowshelf" : i === FREQS.length - 1 ? "highshelf" : "peaking";
        f.frequency.value = frequency;
        f.Q.value = 1.0;
        f.gain.value = 0;
        return f;
      });
      const gain = ctx.createGain();
      const fader = ctx.createGain();
      const limiter = ctx.createDynamicsCompressor();
      limiter.knee.value = 0;
      limiter.attack.value = 0.003;
      limiter.release.value = 0.15;
      let node = source.connect(mono);
      for (const f of filters) node = node.connect(f);
      node.connect(fader).connect(gain).connect(limiter).connect(ctx.destination);
      el.addEventListener("timeupdate", () => {
        const fade = Math.max(0, Math.min(12, Number(settings.fade) || 0));
        if (!fade || !isFinite(el.duration) || el.duration < fade * 3) {
          fader.gain.setTargetAtTime(1, ctx.currentTime, 0.05);
          return;
        }
        const left = el.duration - el.currentTime;
        const level = Math.min(1, el.currentTime / Math.min(2, fade), left / fade);
        fader.gain.setTargetAtTime(Math.max(0, level), ctx.currentTime, 0.08);
      });
      const g = { ctx, filters, gain, limiter, mono };
      graphs.push(g);
      tune(g);
      el.addEventListener("play", () => ctx.state !== "running" && ctx.resume());
    } catch (e) {}
  };

  window.addEventListener("message", e => {
    if (e.source === window.parent && e.data && e.data.rustifyAudio) {
      settings = Object.assign(settings, e.data.rustifyAudio);
      graphs.forEach(tune);
    }
  });

  const play = HTMLMediaElement.prototype.play;
  HTMLMediaElement.prototype.play = function () {
    wire(this);
    return play.apply(this, arguments);
  };
  window.parent.postMessage({ rustifyAudioHello: true }, "*");
})();
