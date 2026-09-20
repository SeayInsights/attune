<!--
  Attune: measure the mic, derive settings, apply them.

  Lives in the Mic tab beside Gate, Equaliser and Compressor, because those are
  exactly the controls it moves. Applying a change here updates those sliders in
  place over the websocket, so the effect is visible without changing screens.
-->
<template>
  <GroupContainer title="Attune" :side-padding="'16px'">
    <div class="col">

      <div class="controls">
        <label for="attune-target">Target</label>
        <select id="attune-target" v-model="target" :disabled="busy">
          <option v-for="t in targets" :key="t.name" :value="t.name">{{ t.name }}</option>
        </select>
      </div>
      <div class="desc" v-if="targetDescription">{{ targetDescription }}</div>

      <div class="controls">
        <label for="attune-seconds">Record</label>
        <input id="attune-seconds" type="number" min="5" max="60"
               v-model.number="seconds" :disabled="busy"/>
        <span class="dim">seconds</span>
      </div>

      <div class="controls">
        <label for="attune-device">Input</label>
        <select id="attune-device" v-model="device" :disabled="busy">
          <option v-for="d in devices" :key="d" :value="d">{{ shortDevice(d) }}</option>
        </select>
      </div>

      <div class="controls buttons">
        <button @click="run(false)" :disabled="busy">Measure</button>
        <button class="accent" @click="run(true)" :disabled="busy">Measure &amp; apply</button>
      </div>

      <div class="rec" v-if="busy">
        <span class="dot"></span>
        <span>{{ recLabel }}</span>
        <span class="bar"><i :style="{ width: progress + '%' }"></i></span>
        <span class="dim count">{{ remaining }}s</span>
      </div>
      <div class="hint" v-else>
        Measure changes nothing. Speak for the whole recording &mdash; it starts
        immediately and does not wait for you.
      </div>

      <div class="scroll" v-if="result || error">
        <div class="bad" v-if="error">{{ error }}</div>

        <template v-if="result">
          <div class="bad" v-if="!result.usable">{{ result.usability }}</div>

          <template v-else>
            <div class="kv" v-for="row in measurementRows" :key="row[0]">
              <span class="dim">{{ row[0] }}</span><span class="num">{{ row[1] }}</span>
            </div>

            <div class="change" v-for="(c, i) in result.recommendation.changes" :key="i">
              <div class="ct">{{ c.setting }}: {{ c.from }} &rarr; {{ c.to }}</div>
              <div class="cw">{{ c.reason }}</div>
            </div>

            <div class="good" v-if="result.recommendation.changes.length === 0">
              Nothing to change &mdash; already matches this target.
            </div>

            <div class="note" v-for="(n, i) in result.recommendation.notes" :key="'n' + i">{{ n }}</div>

            <template v-if="result.applied">
              <div class="good" v-for="(c, i) in result.applied.confirmed" :key="'c' + i">
                Confirmed on device: {{ c }}
              </div>
              <div class="bad" v-for="(f, i) in result.applied.failed" :key="'f' + i">
                Failed: {{ f.setting }} &mdash; {{ f.reason }}
              </div>
            </template>
            <div class="hint" v-else-if="result.recommendation.changes.length">
              Nothing written. Use <strong>Measure &amp; apply</strong> to make these changes.
            </div>
          </template>
        </template>
      </div>

    </div>
  </GroupContainer>
</template>

<script>
import GroupContainer from "@/components/containers/GroupContainer.vue";

export default {
  name: "AttuneTuner",
  components: { GroupContainer },

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
      result: null,
      error: null,
    };
  },

  computed: {
    targetDescription() {
      const t = this.targets.find((x) => x.name === this.target);
      return t ? t.description : "";
    },
    measurementRows() {
      const m = this.result && this.result.measurement;
      if (!m) return [];
      return [
        ["Speech", m.speech_level_dbfs.toFixed(1) + " dBFS"],
        ["Noise floor", m.noise_floor_dbfs.toFixed(1) + " dBFS"],
        ["Peak", m.peak_dbfs.toFixed(1) + " dBFS"],
        ["Signal / noise", m.signal_to_noise_db.toFixed(1) + " dB"],
        ["Crest factor", m.crest_factor_db.toFixed(1) + " dB"],
        ["Speech frames", m.speech_frames + " / " + m.total_frames],
      ];
    },
  },

  mounted() {
    this.loadTargets();
    this.loadDevices();
  },

  beforeUnmount() {
    if (this.timer) clearInterval(this.timer);
  },

  methods: {
    shortDevice(d) {
      // Windows device names carry the driver's decoration. The bus name is
      // the part that identifies it.
      return d.replace(/\s*\(.*\)\s*$/, "");
    },

    async getJSON(url, opts) {
      const r = await fetch(url, opts);
      const body = await r.json().catch(() => ({ error: "Bad response from daemon" }));
      if (!r.ok || body.error) throw new Error(body.error || "HTTP " + r.status);
      return body;
    },

    async loadTargets() {
      try {
        this.targets = await this.getJSON("/api/attune/targets");
        if (this.targets.length) this.target = this.targets[0].name;
      } catch (e) { /* surfaced by the tune call */ }
    },

    async loadDevices() {
      try {
        this.devices = await this.getJSON("/api/attune/devices");
        this.device = this.devices.find((d) => /chat mic/i.test(d)) || this.devices[0] || "";
      } catch (e) { /* surfaced by the tune call */ }
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
        // Applied settings arrive on the sliders beside this box via the
        // daemon's websocket patch; nothing to refresh by hand.
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
.col {
  display: flex;
  flex-direction: column;
  gap: 8px;
  width: 340px;
  color: #fff;
  font-size: 14px;
  text-align: left;
}

.controls { display: flex; align-items: center; gap: 8px; }
.controls.buttons { margin-top: 4px; gap: 6px; }

label { color: #b4bcb8; min-width: 54px; }
.dim { color: #8d9591; font-size: 13px; }
.desc { color: #8d9591; font-size: 12px; margin: -4px 0 2px 62px; line-height: 1.4; }
.num { font-variant-numeric: tabular-nums; }

select, input[type="number"] {
  background-color: #252927; color: #fff;
  border: 1px solid #3b413f; border-radius: 3px;
  padding: 5px 7px; font-family: inherit; font-size: 13px;
  flex: 1; min-width: 0;
}
input[type="number"] { flex: 0 0 64px; }

button {
  flex: 1;
  background-color: #3b413f; color: #fff; border: 0; border-radius: 3px;
  padding: 8px 10px; font-family: inherit; font-size: 13px; cursor: pointer;
}
button.accent { background-color: #59b1b6; color: #0d1817; font-weight: 600; }
button:disabled { opacity: .45; cursor: not-allowed; }

.hint { color: #8d9591; font-size: 12px; line-height: 1.45; }

.rec { display: flex; align-items: center; gap: 8px; font-size: 13px; }
.dot { width: 9px; height: 9px; border-radius: 50%; background: #e0655b;
       animation: attunePulse 1s ease-in-out infinite; flex: 0 0 auto; }
@keyframes attunePulse { 0%,100% { opacity: 1 } 50% { opacity: .25 } }
.bar { flex: 1; height: 4px; background: #252927; border-radius: 2px; overflow: hidden; }
.bar > i { display: block; height: 100%; background: #59b1b6; transition: width .2s linear; }
.count { font-variant-numeric: tabular-nums; flex: 0 0 auto; }

.scroll {
  display: flex; flex-direction: column; gap: 6px;
  max-height: 300px; overflow-y: auto; padding-right: 4px; margin-top: 4px;
}
.scroll::-webkit-scrollbar { width: 5px; }
.scroll::-webkit-scrollbar-track { background: transparent; }
.scroll::-webkit-scrollbar-thumb { background: #3b413f; border-radius: 3px; }

.kv { display: flex; justify-content: space-between; gap: 12px;
      border-bottom: 1px solid #3b413f; padding: 4px 0; font-size: 13px; }

.change, .note {
  border-left: 3px solid #59b1b6; background: #252927;
  padding: 8px 10px; border-radius: 0 3px 3px 0;
}
.note { border-left-color: #d9a441; color: #8d9591; font-size: 12px; line-height: 1.45; }
.ct { font-size: 13px; font-weight: 600; }
.cw { color: #8d9591; font-size: 12px; margin-top: 3px; line-height: 1.45; }

.good { color: #6fc79b; font-size: 13px; }
.bad {
  color: #e0655b; font-size: 12px; line-height: 1.45;
  border-left: 3px solid #e0655b; background: #252927;
  padding: 8px 10px; border-radius: 0 3px 3px 0;
}
</style>
