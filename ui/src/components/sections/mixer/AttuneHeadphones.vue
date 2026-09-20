<!--
  Per-bus headphone correction.

  Lives in the Mixer tab because that is where the buses already are. The GoXLR
  splits Game, Music, Chat and System into separate Windows endpoints in
  hardware, which is the only reason each can carry a different correction at
  the same time.

  The filtering is done by Equalizer APO inside the Windows audio pipeline, so
  nothing here adds latency -- Attune is not in the signal path, it only decides
  what the correction should be.
-->
<template>
  <GroupContainer title="Headphones" :side-padding="'16px'">
    <div class="col">

      <div class="apo" :class="{ missing: state && !state.apo.installed }" v-if="state">
        {{ state.apo.guidance }}
      </div>

      <div class="bad" v-if="error">{{ error }}</div>

      <div class="bus" v-for="bus in buses" :key="bus.name">
        <div class="bhead">
          <span class="bname">{{ bus.name }}</span>
          <span class="bdev" v-if="!bus.device">no matching output device</span>
          <span class="bhead-db" v-else-if="bus.headroom_db > 0.05">
            &minus;{{ bus.headroom_db.toFixed(1) }} dB headroom
          </span>
        </div>

        <select :value="bus.voicing" @change="setVoicing(bus, $event.target.value)"
                :disabled="busy || !bus.device">
          <option v-for="v in voicings" :key="v.name" :value="v.name">
            {{ v.name }} &mdash; {{ v.description }}
          </option>
        </select>

        <svg class="plot" viewBox="0 0 280 54" preserveAspectRatio="none" aria-hidden="true">
          <line x1="0" y1="27" x2="280" y2="27" stroke="#3b413f" stroke-width="1"/>
          <polyline :points="plot(bus.response)" fill="none" stroke="#59b1b6" stroke-width="1.5"/>
        </svg>

        <div class="brow">
          <span class="dim">{{ bus.correction_name || 'no headphone correction' }}</span>
          <button class="link" @click="openImport(bus)" :disabled="busy">
            {{ bus.correction_name ? 'replace' : 'import AutoEQ' }}
          </button>
        </div>
      </div>

      <div class="controls">
        <button class="accent" @click="writeAll" :disabled="busy || !canWrite">
          Apply to Equalizer APO
        </button>
      </div>
      <div class="hint" v-if="result">{{ result }}</div>

      <div class="import" v-if="importing">
        <div class="ititle">Correction for {{ importing.name }}</div>
        <div class="hint">
          Search AutoEQ's published measurements. Attune ships no headphone
          curves &mdash; a correction has to come from a real measurement of your
          model, and inventing one would be worse than having none.
        </div>

        <input v-model="query" @input="runSearch" placeholder="Search, e.g. DT 990 Pro"/>

        <div class="hits" v-if="hits.length">
          <button class="hit" v-for="h in hits" :key="h.path"
                  @click="importFromAutoEq(h)" :disabled="busy">
            <span class="hname">{{ h.name }}</span>
            <span class="hprov">{{ h.provenance }}</span>
          </button>
        </div>
        <div class="hint" v-else-if="searched && query.trim()">
          Nothing matched. The same headphone is often measured by several
          people on different rigs, so try a shorter query.
        </div>

        <details>
          <summary>Paste a curve instead</summary>
          <input v-model="importName" placeholder="Name, e.g. DT 990 Pro"/>
          <textarea v-model="importText" rows="4"
                    placeholder="Preamp: -6.8 dB&#10;Filter 1: ON LSC Fc 105 Hz Gain 5.5 dB Q 0.70&#10;..."></textarea>
          <button class="accent" @click="doImport" :disabled="busy || !importText.trim()">Import pasted</button>
        </details>

        <div class="controls">
          <button @click="closeImport" :disabled="busy">Close</button>
        </div>
      </div>

    </div>
  </GroupContainer>
</template>

<script>
import GroupContainer from "@/components/containers/GroupContainer.vue";

export default {
  name: "AttuneHeadphones",
  components: { GroupContainer },

  data() {
    return {
      state: null,
      buses: [],
      voicings: [],
      busy: false,
      error: null,
      result: null,
      importing: null,
      importName: "",
      importText: "",
      query: "",
      hits: [],
      searched: false,
      searchTimer: null,
    };
  },

  computed: {
    canWrite() {
      return this.state && this.state.apo.installed;
    },
  },

  mounted() {
    this.load();
  },

  methods: {
    async getJSON(url, opts) {
      const r = await fetch(url, opts);
      const body = await r.json().catch(() => ({ error: "Bad response from daemon" }));
      if (!r.ok || body.error) throw new Error(body.error || "HTTP " + r.status);
      return body;
    },

    async load() {
      try {
        const s = await this.getJSON("/api/attune/eq/state");
        this.state = s;
        this.buses = s.buses;
        this.voicings = s.voicings;
        this.error = null;
      } catch (e) {
        this.error = e.message;
      }
    },

    /// Map the response curve to the plot box. +/-12 dB vertical, log frequency
    /// horizontal, because that is how a response is read.
    plot(response) {
      if (!response || !response.length) return "";
      const lo = Math.log(20), hi = Math.log(20000);
      return response
        .map(([hz, db]) => {
          const x = ((Math.log(hz) - lo) / (hi - lo)) * 280;
          const y = 27 - (Math.max(-12, Math.min(12, db)) / 12) * 25;
          return x.toFixed(1) + "," + y.toFixed(1);
        })
        .join(" ");
    },

    async setVoicing(bus, voicing) {
      this.busy = true;
      this.result = null;
      try {
        // Saved but not written: choosing is not the same as changing the
        // system's audio configuration.
        await this.getJSON("/api/attune/eq/apply", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ bus: bus.name, voicing, write: false }),
        });
        await this.load();
      } catch (e) {
        this.error = e.message;
      } finally {
        this.busy = false;
      }
    },

    async writeAll() {
      this.busy = true;
      this.error = null;
      this.result = null;
      try {
        // Any bus carries the whole configuration; the endpoint renders all of
        // them from saved state rather than from this one request.
        const first = this.buses[0];
        const res = await this.getJSON("/api/attune/eq/apply", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ bus: first.name, voicing: first.voicing, write: true }),
        });
        this.result = res.added_include
          ? `Written for ${res.buses} bus(es), and an Include line was added to Equalizer APO's config.`
          : `Written for ${res.buses} bus(es).`;
        await this.load();
      } catch (e) {
        this.error = e.message;
      } finally {
        this.busy = false;
      }
    },

    openImport(bus) {
      this.importing = bus;
      this.importName = "";
      this.importText = "";
      this.query = "";
      this.hits = [];
      this.searched = false;
    },

    closeImport() {
      this.importing = null;
      if (this.searchTimer) clearTimeout(this.searchTimer);
      this.searchTimer = null;
    },

    /// Debounced: the index is searched server-side and a keystroke should not
    /// be a request.
    runSearch() {
      if (this.searchTimer) clearTimeout(this.searchTimer);
      this.searchTimer = setTimeout(async () => {
        const q = this.query.trim();
        if (!q) {
          this.hits = [];
          this.searched = false;
          return;
        }
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

    async importFromAutoEq(hit) {
      this.busy = true;
      this.error = null;
      try {
        await this.getJSON("/api/attune/eq/import-autoeq", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ bus: this.importing.name, path: hit.path }),
        });
        this.closeImport();
        await this.load();
      } catch (e) {
        this.error = e.message;
      } finally {
        this.busy = false;
      }
    },

    async doImport() {
      this.busy = true;
      this.error = null;
      try {
        await this.getJSON("/api/attune/eq/import", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            bus: this.importing.name,
            name: this.importName.trim() || "imported",
            text: this.importText,
          }),
        });
        this.importing = null;
        this.importText = "";
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
.col { display: flex; flex-direction: column; gap: 10px; width: 300px;
       color: #fff; font-size: 14px; text-align: left; }

.apo { color: #8d9591; font-size: 12px; line-height: 1.45;
       border-left: 3px solid #59b1b6; background: #252927;
       padding: 8px 10px; border-radius: 0 3px 3px 0; }
.apo.missing { border-left-color: #d9a441; }

.bus { display: flex; flex-direction: column; gap: 5px;
       border-top: 1px solid #3b413f; padding-top: 8px; }
.bhead { display: flex; justify-content: space-between; align-items: baseline; }
.bname { font-size: 13px; }
.bdev { color: #d9a441; font-size: 11px; }
.bhead-db { color: #8d9591; font-size: 11px; font-variant-numeric: tabular-nums; }

select {
  background-color: #252927; color: #fff;
  border: 1px solid #3b413f; border-radius: 3px;
  padding: 5px 7px; font-family: inherit; font-size: 12px; width: 100%;
}

.plot { width: 100%; height: 54px; background: #252927; border-radius: 3px; }

.brow { display: flex; justify-content: space-between; align-items: center; gap: 8px; }
.dim { color: #8d9591; font-size: 11px; }

button {
  background-color: #3b413f; color: #fff; border: 0; border-radius: 3px;
  padding: 7px 11px; font-family: inherit; font-size: 12px; cursor: pointer;
}
button.accent { background-color: #59b1b6; color: #0d1817; font-weight: 600; }
button.link { background: none; color: #59b1b6; padding: 2px 4px; font-size: 11px; }
button:disabled { opacity: .45; cursor: not-allowed; }

.controls { display: flex; gap: 6px; }
.controls button { flex: 1; }

.hint { color: #8d9591; font-size: 12px; line-height: 1.45; }

.import { display: flex; flex-direction: column; gap: 6px;
          border-top: 1px solid #3b413f; padding-top: 8px; }
.ititle { font-size: 13px; }
input, textarea {
  background-color: #252927; color: #fff;
  border: 1px solid #3b413f; border-radius: 3px;
  padding: 6px 8px; font-family: inherit; font-size: 12px; width: 100%;
  resize: vertical;
}

.hits { display: flex; flex-direction: column; gap: 3px;
        max-height: 190px; overflow-y: auto; }
.hits::-webkit-scrollbar { width: 5px; }
.hits::-webkit-scrollbar-thumb { background: #3b413f; border-radius: 3px; }
.hit { display: flex; flex-direction: column; align-items: flex-start; gap: 1px;
       text-align: left; background: #252927; padding: 6px 9px; }
.hit:hover:not(:disabled) { background: #313734; }
.hname { font-size: 12px; color: #fff; }
.hprov { font-size: 10px; color: #8d9591; }

details { border-top: 1px solid #3b413f; padding-top: 6px; }
summary { color: #8d9591; font-size: 11px; cursor: pointer; margin-bottom: 5px; }
details input, details textarea { margin-bottom: 5px; }

.bad { color: #e0655b; font-size: 12px; line-height: 1.45;
       border-left: 3px solid #e0655b; background: #252927;
       padding: 8px 10px; border-radius: 0 3px 3px 0; }
</style>
