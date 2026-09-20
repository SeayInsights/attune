<!--
  Attune's tab: measure the mic, derive settings, apply them.

  Styled with the same tokens as the rest of the app (#2d3230 panels,
  #3b413f rules, #59b1b6 accent) so it reads as part of the application
  rather than as something bolted on beside it.

  Talks to the Attune endpoints the daemon serves at /api/attune/*.
-->
<template>
  <ContentContainer>
    <ContentBox title="Tune">
      <div class="col">
        <div class="controls">
          <label for="attune-target">Target</label>
          <select id="attune-target" v-model="target" :disabled="busy">
            <option v-for="t in targets" :key="t.name" :value="t.name">
              {{ t.name }} — {{ t.description }}
            </option>
          </select>
        </div>

        <div class="controls">
          <label for="attune-seconds">Record</label>
          <input id="attune-seconds" type="number" min="5" max="60"
                 v-model.number="seconds" :disabled="busy"/>
          <span class="dim">seconds</span>
        </div>

        <div class="controls">
          <label for="attune-device">Input</label>
          <select id="attune-device" v-model="device" :disabled="busy">
            <option v-for="d in devices" :key="d" :value="d">{{ d }}</option>
          </select>
        </div>

        <div class="controls buttons">
          <button @click="run(false)" :disabled="busy">Measure only</button>
          <button class="ghost" @click="run(true)" :disabled="busy">Measure and apply</button>
        </div>

        <p class="hint">
          <strong>Measure only</strong> changes nothing. Speak normally for the whole
          recording — it starts the moment you press the button and does not wait for you.
        </p>

        <div class="rec" v-if="busy">
          <span class="dot"></span>
          <strong>{{ recLabel }}</strong>
          <span class="bar"><i :style="{ width: progress + '%' }"></i></span>
          <span class="dim count">{{ remaining }}s</span>
        </div>
      </div>
    </ContentBox>

    <ContentBox title="Current chain" v-if="chain">
      <div class="col">
        <div class="kv" v-for="row in chainRows" :key="row[0]">
          <span class="dim">{{ row[0] }}</span><span class="num">{{ row[1] }}</span>
        </div>
        <div class="finding" :class="f.severity" v-for="(f, i) in findings" :key="i">
          <div class="ft">{{ f.stage }} — {{ f.summary }}</div>
          <div class="fd">{{ f.detail }}</div>
        </div>
        <div class="good" v-if="chain && findings.length === 0">
          No findings. The chain is coherently configured.
        </div>
      </div>
    </ContentBox>

    <ContentBox title="Result" v-if="result || error">
      <div class="col wide">
        <div class="bad" v-if="error">{{ error }}</div>

        <template v-if="result">
          <div class="bad" v-if="!result.usable">{{ result.usability }}</div>

          <template v-else>
            <div class="kv" v-for="row in measurementRows" :key="row[0]">
              <span class="dim">{{ row[0] }}</span><span class="num">{{ row[1] }}</span>
            </div>

            <div class="change" v-for="(c, i) in result.recommendation.changes" :key="i">
              <div class="ct">{{ c.setting }}: {{ c.from }} → {{ c.to }}</div>
              <div class="cw">{{ c.reason }}</div>
            </div>

            <div class="good" v-if="result.recommendation.changes.length === 0">
              Nothing to change — the chain already matches this target.
            </div>

            <div class="note" v-for="(n, i) in result.recommendation.notes" :key="'n' + i">{{ n }}</div>

            <template v-if="result.applied">
              <div class="good" v-for="(c, i) in result.applied.confirmed" :key="'c' + i">
                Confirmed on device: {{ c }}
              </div>
              <div class="bad" v-for="(f, i) in result.applied.failed" :key="'f' + i">
                Failed: {{ f.setting }} — {{ f.reason }}
              </div>
            </template>

            <p class="hint" v-else-if="result.recommendation.changes.length">
              Nothing was written. Use <strong>Measure and apply</strong> to make these changes.
            </p>
          </template>
        </template>
      </div>
    </ContentBox>
  </ContentContainer>
</template>

<script>
import ContentBox from "@/components/ContentBox.vue";
import ContentContainer from "@/components/containers/ContentContainer.vue";

export default {
  name: "AttuneTab",
  components: { ContentBox, ContentContainer },

  data() {
    return {
      targets: [],
      devices: [],
      target: "streaming",
      seconds: 15,
      device: "",
      busy: false,
      recLabel: "Recording — speak now",
      progress: 0,
      remaining: 0,
      timer: null,
      chain: null,
      findings: [],
      result: null,
      error: null,
    };
  },

  computed: {
    chainRows() {
      const c = this.chain;
      if (!c) return [];
      return [
        ["Mic type", c.mic_type],
        ["Preamp gain", c.gain_db + " dB"],
        ["Gate threshold", c.gate_threshold_db + " dB"],
        ["Compressor threshold", c.compressor_threshold_db + " dB"],
        ["Compressor ratio", c.compressor_ratio],
      ];
    },
    measurementRows() {
      const m = this.result && this.result.measurement;
      if (!m) return [];
      return [
        ["Speech level", m.speech_level_dbfs.toFixed(1) + " dBFS"],
        ["Noise floor", m.noise_floor_dbfs.toFixed(1) + " dBFS"],
        ["Peak", m.peak_dbfs.toFixed(1) + " dBFS"],
        ["Signal to noise", m.signal_to_noise_db.toFixed(1) + " dB"],
        ["Crest factor", m.crest_factor_db.toFixed(1) + " dB"],
        ["Speech frames", m.speech_frames + " of " + m.total_frames],
      ];
    },
  },

  mounted() {
    this.loadState();
    this.loadTargets();
    this.loadDevices();
  },

  beforeUnmount() {
    if (this.timer) clearInterval(this.timer);
  },

  methods: {
    async getJSON(url, opts) {
      const r = await fetch(url, opts);
      const body = await r.json().catch(() => ({ error: "Bad response from daemon" }));
      if (!r.ok || body.error) throw new Error(body.error || "HTTP " + r.status);
      return body;
    },

    async loadState() {
      try {
        const s = await this.getJSON("/api/attune/state");
        this.chain = s.chain;
        this.findings = s.findings;
      } catch (e) {
        this.error = e.message;
      }
    },

    async loadTargets() {
      try {
        this.targets = await this.getJSON("/api/attune/targets");
        if (this.targets.length) this.target = this.targets[0].name;
      } catch (e) { /* the tune call reports it */ }
    },

    async loadDevices() {
      try {
        this.devices = await this.getJSON("/api/attune/devices");
        this.device = this.devices.find((d) => /chat mic/i.test(d)) || this.devices[0] || "";
      } catch (e) { /* the tune call reports it */ }
    },

    startCountdown(seconds) {
      const started = Date.now();
      this.progress = 0;
      this.remaining = seconds;
      this.recLabel = "Recording — speak now";
      this.timer = setInterval(() => {
        const elapsed = (Date.now() - started) / 1000;
        this.progress = Math.min(100, (elapsed / seconds) * 100);
        this.remaining = Math.max(0, Math.ceil(seconds - elapsed));
        if (this.remaining === 0) this.recLabel = "Analysing…";
      }, 100);
    },

    async run(shouldApply) {
      if (this.busy) return;
      this.busy = true;
      this.result = null;
      this.error = null;

      const seconds = Math.max(5, Math.min(60, this.seconds || 15));
      this.startCountdown(seconds);

      try {
        this.result = await this.getJSON("/api/attune/tune", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            seconds,
            target: this.target,
            device: this.device,
            apply: shouldApply,
          }),
        });
        if (shouldApply) this.loadState();
      } catch (e) {
        this.error = e.message;
      } finally {
        clearInterval(this.timer);
        this.timer = null;
        this.busy = false;
      }
    },
  },
};
</script>

<style scoped>
.col { display: flex; flex-direction: column; gap: 10px; min-width: 320px; }
.col.wide { min-width: 420px; max-width: 560px; }

.controls { display: flex; align-items: center; gap: 10px; }
.controls.buttons { margin-top: 6px; }

label { color: #b4bcb8; font-size: 14px; min-width: 56px; }
.dim { color: #8d9591; font-size: 13px; }
.num { font-variant-numeric: tabular-nums; color: #fff; }

select, input[type="number"] {
  background-color: #252927; color: #fff;
  border: 1px solid #3b413f; border-radius: 4px;
  padding: 6px 8px; font-family: inherit; font-size: 14px;
}
input[type="number"] { width: 70px; }

button {
  background-color: #59b1b6; color: #0d1817; border: 0; border-radius: 4px;
  padding: 9px 14px; font-size: 14px; font-weight: 600; cursor: pointer;
}
button.ghost { background-color: #3b413f; color: #fff; }
button:disabled { opacity: .45; cursor: not-allowed; }

.hint { color: #8d9591; font-size: 13px; margin: 4px 0 0; line-height: 1.5; }

.kv { display: flex; justify-content: space-between; gap: 16px;
      border-bottom: 1px solid #3b413f; padding: 5px 0; font-size: 14px; }

.rec { display: flex; align-items: center; gap: 10px; margin-top: 8px; color: #fff; }
.dot { width: 10px; height: 10px; border-radius: 50%; background: #e0655b;
       animation: attunePulse 1s ease-in-out infinite; }
@keyframes attunePulse { 0%,100% { opacity: 1 } 50% { opacity: .25 } }
.bar { flex: 1; height: 5px; background: #252927; border-radius: 3px; overflow: hidden; }
.bar > i { display: block; height: 100%; background: #59b1b6; transition: width .2s linear; }
.count { font-variant-numeric: tabular-nums; }

.change, .note, .finding {
  border-left: 3px solid #59b1b6; background: #252927;
  padding: 10px 12px; border-radius: 0 4px 4px 0;
}
.note { border-left-color: #d9a441; }
.finding.CRIT { border-left-color: #e0655b; }
.finding.WARN { border-left-color: #d9a441; }
.finding.INFO { border-left-color: #59b1b6; }

.ct, .ft { color: #fff; font-size: 14px; font-weight: 600; }
.cw, .fd { color: #8d9591; font-size: 13px; margin-top: 3px; line-height: 1.5; }

.good { color: #6fc79b; font-size: 14px; }
.bad {
  color: #e0655b; font-size: 14px; line-height: 1.5;
  border-left: 3px solid #e0655b; background: #252927;
  padding: 10px 12px; border-radius: 0 4px 4px 0;
}
</style>
