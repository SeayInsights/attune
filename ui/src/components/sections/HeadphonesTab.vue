<!--
  The Headphones tab.

  Laid out like a mixer's equaliser rather than a settings form: one bus at a
  time, a large curve you drag directly, and the processors that follow it as
  separate cards.

  Settings save against the GoXLR profile that is loaded, so they follow it.
  There is no separate save button and no second profile list -- the one at the
  top left is the only one.
-->
<template>
  <div class="tab">

    <!-- Which bus ------------------------------------------------------ -->
    <div class="topbar">
      <div class="bustabs">
        <button v-for="b in buses" :key="b.name"
                class="bustab" :class="{ on: b.name === selectedName }"
                @click="selectedName = b.name">
          {{ b.name }}
          <span class="dot" v-if="!isFlat(b)"></span>
        </button>
      </div>

      <div class="topright">
        <span class="profilechip" :title="'Saved against the ' + profile + ' profile'">
          {{ profile || '…' }}
        </span>
        <button @click="verify" :disabled="busy || !apoReady">Test</button>
        <button class="accent" @click="applyAll" :disabled="busy || !apoReady">Apply</button>
      </div>
    </div>

    <div class="apo" :class="{ missing: apo && !apo.installed }" v-if="apo && !apo.included">
      {{ apo.guidance }}
    </div>
    <div class="bad" v-if="error">{{ error }}</div>
    <div class="note" v-if="status">{{ status }}</div>

    <template v-if="selected">

      <!-- Preset row -------------------------------------------------- -->
      <div class="card row">
        <div class="field">
          <label>Preset</label>
          <select :value="selected.voicing" :disabled="busy"
                  @change="setVoicing($event.target.value)">
            <option v-for="v in voicings" :key="v.name" :value="v.name">{{ v.name }}</option>
          </select>
          <span class="hint">{{ voicingDescription(selected.voicing) }}</span>
        </div>

        <div class="field">
          <label>Headphones</label>
          <div class="inline">
            <span class="value" :class="{ none: !selected.correction_name }">
              {{ shortName(selected.correction_name) || 'none chosen' }}
            </span>
            <button class="ghost" @click="openSearch" :disabled="busy">
              {{ selected.correction_name ? 'Change' : 'Choose' }}
            </button>
            <button class="ghost" v-if="selected.correction_name"
                    @click="clearCorrection" :disabled="busy">Clear</button>
          </div>
          <span class="hint">A measured correction for your model, from AutoEQ.</span>
        </div>
      </div>

      <!-- Equaliser ---------------------------------------------------- -->
      <div class="card">
        <div class="cardhead">
          <span class="cardtitle">Equaliser</span>
          <span class="cardnote" v-if="selected.headroom_db > 0.05">
            &minus;{{ selected.headroom_db.toFixed(1) }} dB headroom applied so it cannot clip
          </span>
          <button class="ghost" @click="resetBands" :disabled="busy">Reset bands</button>
        </div>

        <!-- Region headers, so the plot is readable without knowing Hz -->
        <div class="regions">
          <div class="region" v-for="r in regions" :key="r.label"
               :style="{ flexGrow: r.span }">{{ r.label }}</div>
        </div>

        <div class="plotwrap">
          <div class="ylabels">
            <span>+12 dB</span><span>+6</span><span>0</span><span>&minus;6</span><span>&minus;12</span>
          </div>

          <div class="plotcol">
          <!--
            The viewBox tracks the element's real pixel size rather than being
            fixed. A fixed viewBox with a different aspect ratio is scaled
            uniformly and centred by default, which letterboxes the contents
            into a strip in the middle; forcing preserveAspectRatio="none"
            would instead stretch the band handles into ellipses. Matching the
            box means one unit is one pixel and neither happens.
          -->
          <svg ref="plot" class="plot" :viewBox="`0 0 ${W} ${H}`"
               @pointermove="onDrag" @pointerup="endDrag" @pointerleave="endDrag">
            <!-- grid -->
            <line v-for="g in [0,1,2,3,4]" :key="'h'+g"
                  x1="0" :y1="g*(H/4)" :x2="W" :y2="g*(H/4)"
                  :stroke="g === 2 ? '#4a524e' : '#262b29'" stroke-width="1"/>
            <line v-for="hz in gridFreqs" :key="'v'+hz"
                  :x1="xFor(hz)" y1="0" :x2="xFor(hz)" :y2="H"
                  stroke="#262b29" stroke-width="1"/>

            <!-- flat reference, so the active curve reads as a departure -->
            <line x1="0" :y1="H/2" :x2="W" :y2="H/2" stroke="#6b736f"
                  stroke-width="1.5" stroke-dasharray="4 4"/>

            <!-- the composed response -->
            <polyline :points="responsePoints" fill="none" stroke="#59b1b6" stroke-width="2.5"/>

            <!-- draggable band handles -->
            <g v-for="(hz, i) in bandCentres" :key="hz">
              <circle :cx="xFor(hz)" :cy="yForGain(selected.manual[i])" r="9"
                      :fill="bandColour(i)" :opacity="dragging === i ? 1 : 0.9"
                      class="handle" @pointerdown="startDrag(i, $event)"/>
              <title>{{ label(hz) }}: {{ selected.manual[i].toFixed(1) }} dB</title>
            </g>
          </svg>

          <div class="xlabels">
            <span v-for="hz in gridFreqs" :key="'l'+hz"
                  :style="{ left: (xFor(hz) / W * 100) + '%' }">{{ label(hz) }}</span>
          </div>
          </div>
        </div>

        <div class="hint plothint">
          Drag a dot to change that band. These sit on top of your headphone
          correction and the preset, and survive changing either.
        </div>
      </div>

      <!-- Tone + spatial ---------------------------------------------- -->
      <div class="cards">
        <div class="card">
          <div class="cardhead"><span class="cardtitle">Tone</span>
            <button class="ghost" @click="resetMacros" :disabled="busy">Reset</button>
          </div>
          <div class="macro" v-for="m in ['bass','voice','treble']" :key="m">
            <label>{{ m }}</label>
            <input type="range" :min="-macroLimit" :max="macroLimit" step="0.5"
                   :value="selected.macros[m]" :disabled="busy"
                   @input="onMacro(m, $event.target.value)"/>
            <span class="val">{{ selected.macros[m] > 0 ? '+' : '' }}{{ selected.macros[m].toFixed(1) }}</span>
          </div>
          <div class="hint">Broad controls, for when you want "a bit more bass"
            rather than a specific band.</div>
        </div>

        <div class="card">
          <div class="cardhead"><span class="cardtitle">Spatial</span></div>
          <div class="hint">
            Windows has spatial audio built in &mdash; Windows Sonic for
            Headphones is free and turns surround into a headphone mix. Attune
            does not reimplement it; enable it per device in Windows sound
            settings and it stacks with everything here.
          </div>
          <button class="ghost" @click="openSpatial">Open sound settings</button>
          <div class="hint">
            Crossfeed &mdash; softening headphones' unnaturally hard left/right
            split &mdash; is coming next; it is a different thing from surround
            virtualisation and I would rather label it honestly than call it
            spatial audio.
          </div>
        </div>
      </div>

    </template>

    <!-- Headphone picker ------------------------------------------------ -->
    <div class="modal" v-if="searching" @click.self="searching = false">
      <div class="sheet">
        <div class="stitle">Choose headphones</div>
        <div class="hint">
          Searches AutoEQ's published measurements. The same model measured by
          different people on different rigs gives different corrections, so the
          measurer is shown &mdash; which to trust is a real choice.
        </div>
        <input v-model="query" @input="runSearch" placeholder="e.g. DT 990 Pro"/>
        <label class="check">
          <input type="checkbox" v-model="applyToAll"/>
          Use on all four buses &mdash; you only wear one pair
        </label>

        <div class="hits" v-if="hits.length">
          <button class="hit" v-for="h in hits" :key="h.path"
                  @click="importCorrection(h)" :disabled="busy">
            <span class="hname">{{ h.name }}</span>
            <span class="hprov">{{ h.provenance }}</span>
          </button>
        </div>
        <div class="hint" v-else-if="searched && query.trim()">
          Nothing matched. Try a shorter query &mdash; model names vary.
        </div>

        <button @click="searching = false" :disabled="busy">Close</button>
      </div>
    </div>

  </div>
</template>

<script>
import { store } from "@/store";

const F_MIN = 20;
const F_MAX = 20000;

export default {
  name: "HeadphonesTab",

  data() {
    return {
      // Measured from the element so the viewBox matches it one to one.
      W: 1000,
      H: 200,
      resizeObserver: null,
      profile: null,
      apo: null,
      buses: [],
      voicings: [],
      bandCentres: [],
      limit: 12,
      macroLimit: 10,
      selectedName: "Game",
      dragging: null,
      busy: false,
      error: null,
      status: null,
      searching: false,
      query: "",
      hits: [],
      searched: false,
      searchTimer: null,
      saveTimer: null,
      applyToAll: true,
      gridFreqs: [20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000],
      // Widths are proportional to how much log-frequency each region covers.
      regions: [
        { label: "SUB BASS", span: 1 },
        { label: "BASS", span: 1.3 },
        { label: "LOW MIDS", span: 1 },
        { label: "MID RANGE", span: 1.3 },
        { label: "UPPER MIDS", span: 1.3 },
        { label: "HIGHS", span: 1.6 },
      ],
    };
  },

  computed: {
    selected() {
      return this.buses.find((b) => b.name === this.selectedName) || null;
    },
    apoReady() {
      return this.apo && this.apo.installed;
    },
    responsePoints() {
      const r = this.selected && this.selected.response;
      if (!r || !r.length) return "";
      return r.map(([hz, db]) => `${this.xFor(hz).toFixed(1)},${this.yForGain(db).toFixed(1)}`).join(" ");
    },
    // Settings belong to the loaded GoXLR profile, so a profile change pulls a
    // different set in.
    activeProfile() {
      try {
        return store.getActiveDevice().profile_name;
      } catch (e) {
        return null;
      }
    },
  },

  watch: {
    activeProfile(next, previous) {
      if (next && previous && next !== previous) this.load();
    },
  },

  mounted() {
    this.load();
    this.measure();
    // The plot is fluid, so its size changes with the window and when the
    // sidebar or tab content reflows. Recomputing on resize keeps the viewBox
    // matched to the box rather than only correct at first paint.
    if (typeof ResizeObserver !== "undefined" && this.$refs.plot) {
      this.resizeObserver = new ResizeObserver(() => this.measure());
      this.resizeObserver.observe(this.$refs.plot);
    }
  },

  beforeUnmount() {
    if (this.searchTimer) clearTimeout(this.searchTimer);
    if (this.saveTimer) clearTimeout(this.saveTimer);
    if (this.resizeObserver) this.resizeObserver.disconnect();
  },

  methods: {
    async getJSON(url, opts) {
      const r = await fetch(url, opts);
      const body = await r.json().catch(() => ({ error: "Bad response from daemon" }));
      if (!r.ok || body.error) throw new Error(body.error || "HTTP " + r.status);
      return body;
    },

    // ---- geometry ----

    measure() {
      const el = this.$refs.plot;
      if (!el) return;
      const r = el.getBoundingClientRect();
      if (r.width > 0 && r.height > 0) {
        this.W = Math.round(r.width);
        this.H = Math.round(r.height);
      }
    },

    xFor(hz) {
      const lo = Math.log(F_MIN), hi = Math.log(F_MAX);
      return ((Math.log(hz) - lo) / (hi - lo)) * this.W;
    },
    yForGain(db) {
      const half = this.H / 2;
      return half - (Math.max(-this.limit, Math.min(this.limit, db)) / this.limit) * (half - 10);
    },
    gainForY(y) {
      const half = this.H / 2;
      return ((half - y) / (half - 10)) * this.limit;
    },
    label(hz) {
      return hz >= 1000 ? hz / 1000 + "k" : String(hz);
    },
    bandColour(i) {
      // Spread across the spectrum so a dot's colour hints at its frequency.
      const hue = 280 - (i / (this.bandCentres.length - 1)) * 280;
      return `hsl(${hue}, 65%, 60%)`;
    },
    shortName(n) {
      if (!n) return n;
      return n.length > 42 ? n.slice(0, 40) + "…" : n;
    },
    isFlat(bus) {
      if (!bus) return true;
      const m = bus.macros || {};
      return (
        bus.voicing === "neutral" &&
        !bus.correction_name &&
        (bus.manual || []).every((v) => Math.abs(v) < 0.05) &&
        Math.abs(m.bass || 0) < 0.05 &&
        Math.abs(m.voice || 0) < 0.05 &&
        Math.abs(m.treble || 0) < 0.05
      );
    },
    voicingDescription(name) {
      const v = this.voicings.find((x) => x.name === name);
      return v ? v.description : "";
    },

    // ---- dragging ----

    startDrag(index, event) {
      this.dragging = index;
      event.target.setPointerCapture?.(event.pointerId);
      this.applyDrag(event);
    },
    onDrag(event) {
      if (this.dragging === null) return;
      this.applyDrag(event);
    },
    applyDrag(event) {
      const svg = this.$refs.plot;
      if (!svg) return;
      const rect = svg.getBoundingClientRect();
      // The SVG scales to its box, so translate the pointer into viewBox units.
      const y = ((event.clientY - rect.top) / rect.height) * this.H;
      const gain = Math.max(-this.limit, Math.min(this.limit, this.gainForY(y)));
      this.selected.manual[this.dragging] = Math.round(gain * 2) / 2;
      this.queueSave();
    },
    endDrag() {
      if (this.dragging === null) return;
      this.dragging = null;
      this.queueSave(0);
    },

    // ---- persistence ----

    /// Dragging fires continuously; the local value updates at once so the
    /// curve tracks the pointer, and the save is debounced so one drag is one
    /// request rather than fifty.
    queueSave(delay = 250) {
      if (this.saveTimer) clearTimeout(this.saveTimer);
      this.saveTimer = setTimeout(() => this.save(), delay);
    },

    async save() {
      if (!this.selected) return;
      try {
        await this.getJSON("/api/attune/eq/set", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            bus: this.selected.name,
            voicing: this.selected.voicing,
            manual: this.selected.manual,
            macros: this.selected.macros,
          }),
        });
        await this.load();
      } catch (e) {
        this.error = e.message;
      }
    },

    async load() {
      try {
        const s = await this.getJSON("/api/attune/eq/state");
        this.profile = s.profile;
        this.apo = s.apo;
        this.buses = s.buses;
        this.voicings = s.voicings;
        this.bandCentres = s.band_centres;
        this.limit = s.manual_limit_db;
        this.macroLimit = s.macro_limit_db;
        this.error = null;
      } catch (e) {
        this.error = e.message;
      }
    },

    setVoicing(voicing) {
      this.selected.voicing = voicing;
      this.queueSave(0);
    },
    onMacro(which, value) {
      this.selected.macros[which] = parseFloat(value);
      this.queueSave();
    },
    resetBands() {
      this.selected.manual = this.selected.manual.map(() => 0);
      this.queueSave(0);
    },
    resetMacros() {
      this.selected.macros = { bass: 0, voice: 0, treble: 0 };
      this.queueSave(0);
    },

    // ---- actions ----

    async applyAll() {
      this.busy = true;
      this.error = null;
      this.status = null;
      try {
        const r = await this.getJSON("/api/attune/eq/apply", { method: "POST" });
        this.status = `Applied to ${r.buses} bus(es) for the ${r.profile} profile.`;
        await this.load();
      } catch (e) {
        this.error = e.message;
      } finally {
        this.busy = false;
      }
    },

    async verify() {
      this.busy = true;
      this.error = null;
      this.status = `Playing test noise through ${this.selectedName} for a few seconds…`;
      try {
        const r = await this.getJSON("/api/attune/eq/verify", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ bus: this.selectedName, seconds: 6 }),
        });
        const at = (hz) => {
          const b = r.bands.find((x) => x.centre_hz === hz);
          return b ? b.level_db.toFixed(1) : "?";
        };
        this.status =
          `Measured ${this.selectedName}: ${at(125)} dB at 125 Hz, ${at(4000)} dB at 4 kHz. ` +
          `Run it with a curve on and off to see the difference.`;
      } catch (e) {
        this.error = e.message;
      } finally {
        this.busy = false;
      }
    },

    openSpatial() {
      window.open("ms-settings:sound", "_blank");
    },

    // ---- headphone picker ----

    openSearch() {
      this.searching = true;
      this.query = "";
      this.hits = [];
      this.searched = false;
    },
    runSearch() {
      if (this.searchTimer) clearTimeout(this.searchTimer);
      this.searchTimer = setTimeout(async () => {
        const q = this.query.trim();
        if (!q) { this.hits = []; this.searched = false; return; }
        try {
          this.hits = await this.getJSON("/api/attune/eq/search?q=" + encodeURIComponent(q));
          this.error = null;
        } catch (e) {
          this.hits = [];
          this.error = e.message;
        } finally {
          this.searched = true;
        }
      }, 250);
    },
    async importCorrection(hit) {
      this.busy = true;
      try {
        await this.getJSON("/api/attune/eq/import-autoeq", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            bus: this.selectedName,
            path: hit.path,
            all_buses: this.applyToAll,
          }),
        });
        this.searching = false;
        await this.load();
      } catch (e) {
        this.error = e.message;
      } finally {
        this.busy = false;
      }
    },
    async clearCorrection() {
      this.busy = true;
      try {
        await this.getJSON("/api/attune/eq/clear-correction", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ bus: this.selectedName }),
        });
        await this.load();
      } catch (e) {
        this.error = e.message;
      } finally {
        this.busy = false;
      }
    },
  },
};
</script>

<style scoped>
.tab { padding: 22px 30px 50px; color: #fff; text-align: left; }

.topbar { display: flex; justify-content: space-between; align-items: center;
          gap: 16px; margin-bottom: 14px; flex-wrap: wrap; }
.bustabs { display: flex; gap: 2px; }
.bustab { position: relative; background: none; color: #8d9591; border: 0;
          border-bottom: 2px solid transparent; padding: 8px 18px;
          font-family: inherit; font-size: 14px; cursor: pointer; }
.bustab.on { color: #fff; border-bottom-color: #59b1b6; }
.bustab .dot { position: absolute; top: 6px; right: 6px; width: 5px; height: 5px;
               border-radius: 50%; background: #59b1b6; }

.topright { display: flex; align-items: center; gap: 8px; }
.profilechip { background: #252927; color: #8d9591; font-size: 11px;
               padding: 5px 9px; border-radius: 10px; }

.card { background: #2d3230; padding: 16px 18px; margin-bottom: 12px; }
.cards { display: flex; gap: 12px; flex-wrap: wrap; }
.cards .card { flex: 1 1 340px; margin-bottom: 0; }

.card.row { display: flex; gap: 28px; flex-wrap: wrap; }
.field { display: flex; flex-direction: column; gap: 5px; min-width: 260px; }
.field label, .cardtitle { font-size: 11px; text-transform: uppercase;
                           letter-spacing: .7px; color: #b4bcb8; }
.inline { display: flex; align-items: center; gap: 6px; }
.value { flex: 1; font-size: 13px; }
.value.none { color: #8d9591; font-style: italic; }

.cardhead { display: flex; align-items: center; gap: 12px; margin-bottom: 12px; }
.cardnote { color: #8d9591; font-size: 11px; flex: 1; }
.cardhead .ghost { margin-left: auto; }

.regions { display: flex; gap: 2px; margin-bottom: 6px; }
.region { background: #252927; color: #8d9591; font-size: 10px;
          letter-spacing: .6px; text-align: center; padding: 5px 0; }

.plotwrap { display: flex; gap: 8px; }
.ylabels { display: flex; flex-direction: column; justify-content: space-between;
           color: #6b736f; font-size: 10px; height: 220px; padding: 2px 0;
           font-variant-numeric: tabular-nums; flex: 0 0 auto; }
.plot { display: block; width: 100%; height: 220px; background: #252927; touch-action: none; }
.handle { cursor: ns-resize; }
.handle:hover { r: 11; }

.plotcol { flex: 1; display: flex; flex-direction: column; min-width: 0; }
.xlabels { position: relative; height: 16px; margin-top: 3px; }
.xlabels span { position: absolute; transform: translateX(-50%);
                color: #6b736f; font-size: 10px; }
.plothint { margin-top: 6px; }

.macro { display: flex; align-items: center; gap: 10px; margin-bottom: 7px; }
.macro label { width: 46px; text-transform: capitalize; color: #b4bcb8;
               font-size: 12px; letter-spacing: 0; }
.macro input[type="range"] { flex: 1; accent-color: #59b1b6; }
.macro .val { width: 38px; text-align: right; color: #59b1b6; font-size: 11px;
              font-variant-numeric: tabular-nums; }

select, input[type="text"], .sheet input:not([type="checkbox"]) {
  background-color: #252927; color: #fff; border: 1px solid #3b413f;
  border-radius: 3px; padding: 7px 9px; font-family: inherit; font-size: 13px; width: 100%;
}

button { background-color: #3b413f; color: #fff; border: 0; border-radius: 3px;
         padding: 8px 14px; font-family: inherit; font-size: 13px; cursor: pointer; }
button.accent { background-color: #59b1b6; color: #0d1817; font-weight: 600; }
button.ghost { background: none; border: 1px solid #3b413f; color: #b4bcb8;
               padding: 5px 10px; font-size: 12px; }
button:disabled { opacity: .45; cursor: not-allowed; }

.hint { color: #8d9591; font-size: 11px; line-height: 1.5; }

.apo, .note { color: #8d9591; font-size: 12px; line-height: 1.45;
              border-left: 3px solid #59b1b6; background: #2d3230;
              padding: 9px 12px; border-radius: 0 3px 3px 0; margin-bottom: 12px; }
.apo.missing { border-left-color: #d9a441; }
.bad { color: #e0655b; font-size: 12px; line-height: 1.45;
       border-left: 3px solid #e0655b; background: #2d3230;
       padding: 9px 12px; border-radius: 0 3px 3px 0; margin-bottom: 12px; }

.modal { position: fixed; inset: 0; background: rgba(0,0,0,.6); z-index: 50;
         display: flex; align-items: center; justify-content: center; }
.sheet { background: #2d3230; padding: 22px; width: 460px; max-height: 76vh;
         display: flex; flex-direction: column; gap: 10px; overflow-y: auto; }
.stitle { font-size: 16px; }
.check { display: flex; align-items: center; gap: 8px; color: #8d9591; font-size: 12px; }
.check input { width: auto; }
.hits { display: flex; flex-direction: column; gap: 3px; max-height: 280px; overflow-y: auto; }
.hit { display: flex; flex-direction: column; align-items: flex-start; gap: 1px;
       text-align: left; background: #252927; padding: 8px 11px; }
.hit:hover:not(:disabled) { background: #343b38; }
.hname { font-size: 12px; }
.hprov { font-size: 10px; color: #8d9591; }
</style>
