<!--
  The Headphones tab.

  Its own tab rather than a panel in the Mixer, because it is a whole subject:
  what each output bus does to what you hear.

  Settings here are saved against the GoXLR profile that is currently loaded, so
  they follow it. There is no separate save button and no second profile list --
  the one at the top left is the only one.
-->
<template>
  <div class="tab">

    <!-- What this is, because it is not obvious from a row of sliders. -->
    <div class="intro">
      <div class="lead">
        Your GoXLR sends Game, Music, Chat and System to Windows as four separate
        devices. That means each one can be equalised differently, at the same
        time &mdash; a lean curve for hearing footsteps while music stays full.
      </div>
      <div class="sub">
        Nothing here touches your microphone, and nothing adds delay: the
        filtering runs inside the Windows audio pipeline, not inside Attune.
        Changes save against the <strong>{{ profile || '…' }}</strong> profile
        and follow it when you switch.
      </div>
    </div>

    <div class="apo" :class="{ missing: apo && !apo.installed }" v-if="apo">
      {{ apo.guidance }}
    </div>
    <div class="bad" v-if="error">{{ error }}</div>

    <div class="bar">
      <button class="accent" @click="applyAll" :disabled="busy || !apo || !apo.installed">
        Apply to my headphones
      </button>
      <button @click="verify" :disabled="busy || !apo || !apo.installed">
        Verify it's working
      </button>
      <span class="status" v-if="status">{{ status }}</span>
    </div>

    <div class="buses">
      <div class="bus" v-for="bus in buses" :key="bus.name">

        <div class="bhead">
          <span class="bname">{{ bus.name }}</span>
          <span class="bwarn" v-if="!bus.device">no matching Windows device</span>
          <span class="bdim" v-else-if="bus.headroom_db > 0.05">
            &minus;{{ bus.headroom_db.toFixed(1) }} dB headroom
          </span>
        </div>

        <!-- 1. Which headphones -->
        <div class="layer">
          <div class="ltitle">1 &middot; Your headphones</div>
          <div class="lwhy">A measured correction for your exact model, undoing
            what is wrong with them. Attune does not invent these.</div>
          <div class="lrow">
            <span class="lval" :class="{ none: !bus.correction_name }">
              {{ bus.correction_name || 'none set' }}
            </span>
            <button class="link" @click="openSearch(bus)" :disabled="busy">
              {{ bus.correction_name ? 'change' : 'choose' }}
            </button>
            <button class="link" v-if="bus.correction_name"
                    @click="clearCorrection(bus)" :disabled="busy">clear</button>
          </div>
        </div>

        <!-- 2. What for -->
        <div class="layer">
          <div class="ltitle">2 &middot; What this bus is for</div>
          <div class="lwhy">{{ voicingDescription(bus.voicing) }}</div>
          <select :value="bus.voicing" :disabled="busy"
                  @change="setVoicing(bus, $event.target.value)">
            <option v-for="v in voicings" :key="v.name" :value="v.name">{{ v.name }}</option>
          </select>
        </div>

        <!-- 3. Your own adjustments -->
        <div class="layer">
          <div class="ltitle">3 &middot; Your own adjustments</div>
          <div class="lwhy">Trim any band by ear. These sit on top of the two
            above and survive changing either.</div>

          <div class="band" v-for="(hz, i) in bandCentres" :key="hz">
            <span class="bhz">{{ label(hz) }}</span>
            <input type="range" :min="-limit" :max="limit" step="0.5"
                   :value="bus.manual[i]" :disabled="busy"
                   @input="onManual(bus, i, $event.target.value)"/>
            <span class="bdb" :class="{ zero: Math.abs(bus.manual[i]) < 0.05 }">
              {{ bus.manual[i] > 0 ? '+' : '' }}{{ bus.manual[i].toFixed(1) }}
            </span>
          </div>

          <button class="link flat" @click="resetManual(bus)" :disabled="busy">
            reset all bands
          </button>
        </div>

        <!-- The result -->
        <div class="layer">
          <div class="ltitle">Result</div>
          <svg class="plot" viewBox="0 0 300 80" preserveAspectRatio="none" aria-hidden="true">
            <line v-for="g in [20,40,60]" :key="g" x1="0" :y1="g" x2="300" :y2="g"
                  stroke="#2b302e" stroke-width="1"/>
            <line x1="0" y1="40" x2="300" y2="40" stroke="#3b413f" stroke-width="1"/>
            <polyline :points="plot(bus.response)" fill="none" stroke="#59b1b6" stroke-width="2"/>
          </svg>
          <div class="axis"><span>20 Hz</span><span>1 kHz</span><span>20 kHz</span></div>
        </div>

      </div>
    </div>

    <!-- Headphone picker -->
    <div class="modal" v-if="searching" @click.self="searching = null">
      <div class="sheet">
        <div class="stitle">Choose headphones for {{ searching.name }}</div>
        <div class="lwhy">
          Searches AutoEQ's published measurements. The same model measured by
          different people on different rigs gives different corrections, so the
          measurer is shown &mdash; which to trust is a real choice.
        </div>
        <input v-model="query" @input="runSearch" placeholder="e.g. DT 990 Pro" autofocus/>
        <label class="allbuses">
          <input type="checkbox" v-model="applyToAll"/>
          Use for all four buses (you only have one pair of headphones on)
        </label>

        <div class="hits" v-if="hits.length">
          <button class="hit" v-for="h in hits" :key="h.path"
                  @click="importCorrection(h)" :disabled="busy">
            <span class="hname">{{ h.name }}</span>
            <span class="hprov">{{ h.provenance }}</span>
          </button>
        </div>
        <div class="lwhy" v-else-if="searched && query.trim()">
          Nothing matched. Try a shorter query &mdash; model names vary.
        </div>

        <button @click="searching = null" :disabled="busy">Close</button>
      </div>
    </div>

  </div>
</template>

<script>
import { store } from "@/store";

export default {
  name: "HeadphonesTab",

  data() {
    return {
      profile: null,
      apo: null,
      buses: [],
      voicings: [],
      bandCentres: [],
      limit: 12,
      busy: false,
      error: null,
      status: null,
      searching: null,
      query: "",
      hits: [],
      searched: false,
      searchTimer: null,
      saveTimer: null,
      applyToAll: true,
    };
  },

  computed: {
    // Settings belong to the loaded GoXLR profile, so a profile change has to
    // pull a different set of settings in.
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
  },

  beforeUnmount() {
    if (this.searchTimer) clearTimeout(this.searchTimer);
    if (this.saveTimer) clearTimeout(this.saveTimer);
  },

  methods: {
    async getJSON(url, opts) {
      const r = await fetch(url, opts);
      const body = await r.json().catch(() => ({ error: "Bad response from daemon" }));
      if (!r.ok || body.error) throw new Error(body.error || "HTTP " + r.status);
      return body;
    },

    label(hz) {
      return hz >= 1000 ? hz / 1000 + "k" : String(hz);
    },

    voicingDescription(name) {
      const v = this.voicings.find((x) => x.name === name);
      return v ? v.description : "";
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
        this.error = null;
      } catch (e) {
        this.error = e.message;
      }
    },

    plot(response) {
      if (!response || !response.length) return "";
      const lo = Math.log(20), hi = Math.log(20000);
      return response
        .map(([hz, db]) => {
          const x = ((Math.log(hz) - lo) / (hi - lo)) * 300;
          const y = 40 - (Math.max(-20, Math.min(20, db)) / 20) * 38;
          return x.toFixed(1) + "," + y.toFixed(1);
        })
        .join(" ");
    },

    async setVoicing(bus, voicing) {
      bus.voicing = voicing;
      await this.save(bus);
    },

    /// Dragging a slider fires continuously; the local value updates at once so
    /// the plot tracks, and the save is debounced so a drag is one request.
    onManual(bus, index, value) {
      bus.manual[index] = parseFloat(value);
      if (this.saveTimer) clearTimeout(this.saveTimer);
      this.saveTimer = setTimeout(() => this.save(bus), 200);
    },

    async resetManual(bus) {
      bus.manual = bus.manual.map(() => 0);
      await this.save(bus);
    },

    async save(bus) {
      this.status = null;
      try {
        await this.getJSON("/api/attune/eq/set", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            bus: bus.name,
            voicing: bus.voicing,
            manual: bus.manual,
          }),
        });
        await this.load();
      } catch (e) {
        this.error = e.message;
      }
    },

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
      this.status = "Playing test noise through Game for a few seconds…";
      try {
        const r = await this.getJSON("/api/attune/eq/verify", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ bus: "Game", seconds: 6 }),
        });
        const low = r.bands.find((b) => b.centre_hz === 125);
        const high = r.bands.find((b) => b.centre_hz === 4000);
        this.status =
          `Measured the Game bus: ${low ? low.level_db.toFixed(1) : "?"} dB at 125 Hz, ` +
          `${high ? high.level_db.toFixed(1) : "?"} dB at 4 kHz. ` +
          `Run this with a curve on and off to see the difference.`;
      } catch (e) {
        this.error = e.message;
      } finally {
        this.busy = false;
      }
    },

    openSearch(bus) {
      this.searching = bus;
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
            bus: this.searching.name,
            path: hit.path,
            all_buses: this.applyToAll,
          }),
        });
        this.searching = null;
        await this.load();
      } catch (e) {
        this.error = e.message;
      } finally {
        this.busy = false;
      }
    },

    async clearCorrection(bus) {
      this.busy = true;
      try {
        await this.getJSON("/api/attune/eq/clear-correction", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ bus: bus.name }),
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
.tab { padding: 30px 40px 50px; color: #fff; text-align: left; }

.intro { max-width: 900px; margin-bottom: 18px; }
.lead { font-size: 15px; line-height: 1.5; }
.sub { color: #8d9591; font-size: 13px; line-height: 1.5; margin-top: 6px; }

.apo { max-width: 900px; color: #8d9591; font-size: 13px; line-height: 1.45;
       border-left: 3px solid #59b1b6; background: #2d3230;
       padding: 10px 12px; border-radius: 0 3px 3px 0; margin-bottom: 14px; }
.apo.missing { border-left-color: #d9a441; }

.bar { display: flex; align-items: center; gap: 10px; margin-bottom: 20px; }
.status { color: #8d9591; font-size: 13px; }

.buses { display: flex; flex-direction: row; gap: 15px; overflow-x: auto; padding-bottom: 12px; }
.buses::-webkit-scrollbar { height: 6px; }
.buses::-webkit-scrollbar-thumb { background: #3b413f; border-radius: 3px; }

.bus { flex: 0 0 300px; background: #2d3230; padding: 16px; display: flex;
       flex-direction: column; gap: 14px; }

.bhead { display: flex; justify-content: space-between; align-items: baseline;
         border-bottom: 1px solid #3b413f; padding-bottom: 8px; }
.bname { font-size: 15px; text-transform: uppercase; letter-spacing: .5px; }
.bwarn { color: #d9a441; font-size: 11px; }
.bdim { color: #8d9591; font-size: 11px; font-variant-numeric: tabular-nums; }

.layer { display: flex; flex-direction: column; gap: 5px; }
.ltitle { font-size: 12px; text-transform: uppercase; letter-spacing: .6px; color: #b4bcb8; }
.lwhy { color: #8d9591; font-size: 11px; line-height: 1.45; }
.lrow { display: flex; align-items: center; gap: 6px; }
.lval { flex: 1; font-size: 12px; }
.lval.none { color: #8d9591; font-style: italic; }

select, input[type="text"], .modal input:not([type="checkbox"]):not([type="range"]) {
  background-color: #252927; color: #fff; border: 1px solid #3b413f;
  border-radius: 3px; padding: 6px 8px; font-family: inherit; font-size: 12px; width: 100%;
}

button {
  background-color: #3b413f; color: #fff; border: 0; border-radius: 3px;
  padding: 8px 12px; font-family: inherit; font-size: 13px; cursor: pointer;
}
button.accent { background-color: #59b1b6; color: #0d1817; font-weight: 600; }
button.link { background: none; color: #59b1b6; padding: 2px 4px; font-size: 11px; }
button.link.flat { align-self: flex-start; margin-top: 2px; }
button:disabled { opacity: .45; cursor: not-allowed; }

.band { display: flex; align-items: center; gap: 7px; }
.bhz { width: 34px; text-align: right; color: #8d9591; font-size: 10px;
       font-variant-numeric: tabular-nums; }
.bdb { width: 32px; text-align: right; font-size: 10px; color: #59b1b6;
       font-variant-numeric: tabular-nums; }
.bdb.zero { color: #6b736f; }

input[type="range"] { flex: 1; height: 3px; accent-color: #59b1b6;
                      background: #252927; cursor: pointer; }

.plot { width: 100%; height: 80px; background: #252927; border-radius: 3px; }
.axis { display: flex; justify-content: space-between; color: #6b736f; font-size: 9px; }

.modal { position: fixed; inset: 0; background: rgba(0,0,0,.6);
         display: flex; align-items: center; justify-content: center; z-index: 50; }
.sheet { background: #2d3230; padding: 20px; width: 440px; max-height: 76vh;
         display: flex; flex-direction: column; gap: 9px; overflow-y: auto; }
.stitle { font-size: 15px; }
.allbuses { display: flex; align-items: center; gap: 7px; color: #8d9591; font-size: 12px; }
.allbuses input { width: auto; }

.hits { display: flex; flex-direction: column; gap: 3px; max-height: 260px; overflow-y: auto; }
.hit { display: flex; flex-direction: column; align-items: flex-start; gap: 1px;
       text-align: left; background: #252927; padding: 7px 10px; }
.hit:hover:not(:disabled) { background: #343b38; }
.hname { font-size: 12px; }
.hprov { font-size: 10px; color: #8d9591; }

.bad { max-width: 900px; color: #e0655b; font-size: 13px; line-height: 1.45;
       border-left: 3px solid #e0655b; background: #2d3230;
       padding: 10px 12px; border-radius: 0 3px 3px 0; margin-bottom: 14px; }
</style>
