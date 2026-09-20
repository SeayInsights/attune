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
  <div class="hptab">

    <!-- Which bus ------------------------------------------------------ -->
    <div class="topbar">
      <div class="bustabs">
        <button v-for="b in buses" :key="b.name"
                class="bustab" :class="{ on: b.name === selectedName }"
                @click="selectedName = b.name">
          {{ b.name }}
          <span class="dot" v-if="!isFlat(b)"></span>
          <!-- Live level, read off the bus through Windows loopback. -->
          <span class="meter" :class="{ clip: meterFor(b.name).clipped }">
            <span class="rms" :style="{ width: meterWidth(meterFor(b.name).rms_dbfs) }"></span>
            <span class="peak" :style="{ left: meterWidth(meterFor(b.name).peak_dbfs) }"></span>
          </span>
        </button>
      </div>

      <div class="topright">
        <span class="profilechip" :title="'Saved against the ' + profile + ' profile'">
          {{ profile || '…' }}
        </span>
        <!--
          Hold to hear it without Attune. Press-and-hold rather than a toggle
          because comparing is the job: you cannot judge a change you have to
          click twice to undo, and a toggle left on is a silent way to have no
          correction at all.
        -->
        <button class="compare" :class="{ held: bypassed }"
                :disabled="busy || !apoReady"
                title="Hold to hear the channels without any of this"
                @pointerdown="setBypass(true)" @pointerup="setBypass(false)"
                @pointerleave="setBypass(false)"
                @keydown.space.prevent="setBypass(true)"
                @keyup.space.prevent="setBypass(false)">
          {{ bypassed ? 'Bypassed' : 'Hold to compare' }}
        </button>
        <button @click="verify" :disabled="busy || !apoReady">Test</button>
        <button class="accent" @click="applyAll" :disabled="busy || !apoReady">Apply</button>
      </div>
    </div>

    <!--
      Shown when APO is missing, when it is installed but not yet loading these
      curves, and -- the case that matters most -- when it is loading nothing
      because an endpoint was detached. That last one is invisible otherwise:
      the curves are still drawn, the settings are still saved, and the audio
      is flat.
    -->
    <div class="apo"
         :class="{ missing: apo && !apo.installed, wiped: apo && apo.installed && !apo.attached }"
         v-if="apo && (!apo.included || !apo.attached)">
      {{ apo.guidance }}
    </div>
    <!--
      Anything Equalizer APO loads before Attune's own config is already in the
      signal by the time a correction runs, so a measurement will not match the
      curve on screen and nothing on screen would explain why.
    -->
    <div class="apo missing" v-if="interference.length">
      Equalizer APO is loading this before Attune, so it colours everything
      here: <code>{{ interference.join(' · ') }}</code>. Comment those lines out
      in Equalizer APO&rsquo;s <code>config.txt</code> for measurements that
      match what you see.
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
          <!-- Positioned by the same function as the grid lines and the
               handles, so a label always names the line beside it. -->
          <div class="ylabels">
            <span v-for="db in gainGrid" :key="db" :style="{ top: yPct(db) + '%' }">
              {{ db > 0 ? '+' : db < 0 ? '−' : '' }}{{ Math.abs(db) }}{{ db === limit ? ' dB' : '' }}
            </span>
          </div>

          <div class="plotcol">
          <!--
            preserveAspectRatio="none" so the grid and curve always span the
            box, whatever its shape. That distorts anything that must stay
            round, so the band handles are HTML positioned in percent over the
            top rather than SVG circles inside it. Measuring the element and
            matching the viewBox was tried first and kept letterboxing whenever
            layout settled after the measurement.
          -->
          <div class="plotbox" ref="plot">
            <svg class="plot" viewBox="0 0 1000 400" preserveAspectRatio="none">
              <line v-for="db in gainGrid" :key="'h'+db"
                    x1="0" :y1="yForGain(db)" x2="1000" :y2="yForGain(db)"
                    stroke="#262b29" vector-effect="non-scaling-stroke" stroke-width="1"/>
              <line v-for="hz in gridFreqs" :key="'v'+hz"
                    :x1="xPct(hz)*10" y1="0" :x2="xPct(hz)*10" y2="400"
                    stroke="#262b29" vector-effect="non-scaling-stroke" stroke-width="1"/>
              <line x1="0" :y1="yForGain(0)" x2="1000" :y2="yForGain(0)" stroke="#6b736f"
                    stroke-dasharray="4 4" vector-effect="non-scaling-stroke" stroke-width="1.5"/>
              <polyline :points="responsePoints" fill="none" stroke="#59b1b6"
                        vector-effect="non-scaling-stroke" stroke-width="2.5"
                        stroke-linejoin="round"/>
            </svg>

            <button v-for="(hz, i) in bandCentres" :key="hz" class="handle"
                    :class="{ dragging: dragging === i }"
                    :style="{ left: xPct(hz) + '%', top: yPct(selected.manual[i]) + '%',
                              background: bandColour(i) }"
                    :title="label(hz) + ': ' + selected.manual[i].toFixed(1) + ' dB'"
                    :aria-label="label(hz) + ' band, ' + selected.manual[i].toFixed(1) + ' decibels'"
                    @pointerdown="startDrag(i, $event)"
                    @keydown.up.prevent="nudge(i, 0.5)"
                    @keydown.down.prevent="nudge(i, -0.5)"></button>
          </div>

          <div class="xlabels">
            <span v-for="hz in gridFreqs" :key="'l'+hz"
                  :style="{ left: xPct(hz) + '%' }">{{ label(hz) }}</span>
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
          <div class="cardhead">
            <span class="cardtitle">Spatial</span>
            <span class="cardnote" v-if="spatialBus">
              {{ selectedName }} is on {{ spatialBus.active_label }}
            </span>
          </div>

          <template v-if="spatial && spatial.supported && spatialBus">
            <div class="formats">
              <button v-for="f in spatialBus.formats" :key="f.subtype"
                      class="format"
                      :class="{ on: f.subtype === spatialBus.active, off: !f.usable }"
                      :disabled="spatialBusy || !f.usable"
                      :title="f.usable ? f.note : f.label + ' is not installed. ' + f.note"
                      @click="setSpatial(f)">
                <span class="fname">
                  {{ f.label }}
                  <span class="ftag" v-if="!f.usable">not installed</span>
                </span>
                <span class="fnote">{{ f.note }}</span>
              </button>
            </div>

            <label class="check">
              <input type="checkbox" v-model="spatialAllBuses"/>
              Set on all 4 channels &mdash; otherwise the mix changes when you
              alt-tab
            </label>

            <div class="bad" v-if="spatialError">{{ spatialError }}</div>

            <div class="hint">
              This is Windows' own virtualiser, switched through the supported
              API rather than reimplemented. Dolby and DTS are listed because
              Windows knows their names, and greyed out when whatever provides
              them is not installed &mdash; Windows will accept a change to one
              of those and then quietly ignore it, so there is nothing to wait
              for.
            </div>
          </template>

          <div class="hint" v-else>
            {{ spatial && !spatial.supported
               ? 'Windows spatial audio is a Windows feature.'
               : 'Reading spatial audio state…' }}
          </div>

        </div>
      </div>

      <!-- Crossfeed + loudness ---------------------------------------- -->
      <div class="cards">
        <div class="card">
          <div class="cardhead">
            <span class="cardtitle">Crossfeed</span>
            <span class="cardnote" v-if="busExtras.crossfeed">
              &minus;{{ crossfeedHeadroom.toFixed(1) }} dB so it cannot clip
            </span>
          </div>

          <div class="formats">
            <button class="format" :class="{ on: !busExtras.crossfeed }"
                    :disabled="extrasBusy" @click="setCrossfeed(null)">
              <span class="fname">Off</span>
              <span class="fnote">Hard left/right, the way headphones do it.</span>
            </button>
            <button v-for="p in crossfeedPresets" :key="p.name" class="format"
                    :class="{ on: presetActive(p) }"
                    :disabled="extrasBusy" @click="setCrossfeed(p.settings)">
              <span class="fname">{{ p.name }}</span>
              <span class="fnote">{{ p.description }}</span>
            </button>
          </div>

          <template v-if="busExtras.crossfeed">
            <div class="macro">
              <label>level</label>
              <input type="range" :min="limits.level_db[0]" :max="limits.level_db[1]" step="0.5"
                     :value="busExtras.crossfeed.level_db" :disabled="extrasBusy"
                     @change="tweakCrossfeed('level_db', $event.target.value)"/>
              <span class="val">{{ busExtras.crossfeed.level_db.toFixed(1) }}</span>
            </div>
            <div class="macro">
              <label>cutoff</label>
              <input type="range" :min="limits.cutoff_hz[0]" :max="limits.cutoff_hz[1]" step="25"
                     :value="busExtras.crossfeed.cutoff_hz" :disabled="extrasBusy"
                     @change="tweakCrossfeed('cutoff_hz', $event.target.value)"/>
              <span class="val">{{ busExtras.crossfeed.cutoff_hz.toFixed(0) }}</span>
            </div>
            <div class="macro">
              <label>delay</label>
              <input type="range" :min="limits.delay_us[0]" :max="limits.delay_us[1]" step="10"
                     :value="busExtras.crossfeed.delay_us" :disabled="extrasBusy"
                     @change="tweakCrossfeed('delay_us', $event.target.value)"/>
              <span class="val">{{ busExtras.crossfeed.delay_us.toFixed(0) }}</span>
            </div>
          </template>

          <label class="check">
            <input type="checkbox" v-model="extrasAllBuses"/>
            Set on all 4 channels
          </label>
          <div class="hint">
            On speakers your left ear hears the right speaker, later and duller,
            because your head is in the way. Headphones remove that, so
            hard-panned mixes sit inside one ear. This puts a little of it back.
            It is not surround &mdash; it will not help you hear someone behind
            you, which is what the Spatial card is for.
          </div>
        </div>

        <div class="card">
          <div class="cardhead">
            <span class="cardtitle">Loudness</span>
            <span class="cardnote" v-if="pluginLoaded === false">
              Equalizer APO did not load it
            </span>
          </div>

          <div class="formats" v-if="plugins.length">
            <button class="format" :class="{ on: !busExtras.plugin }"
                    :disabled="extrasBusy" @click="setPlugin(null)">
              <span class="fname">Off</span>
            </button>
            <button v-for="p in plugins" :key="p" class="format"
                    :class="{ on: busExtras.plugin === p }"
                    :disabled="extrasBusy" @click="setPlugin(p)">
              <span class="fname">{{ p }}</span>
            </button>
          </div>

          <div class="hint">{{ pluginGuidance }}</div>
        </div>
      </div>

      <!-- Per-game profiles ------------------------------------------- -->
      <div class="card">
        <div class="cardhead">
          <span class="cardtitle">Per-game profiles</span>
          <span class="cardnote" v-if="foreground">
            in front right now: {{ foreground }}
          </span>
          <label class="check switch">
            <input type="checkbox" :checked="autoswitch.enabled" :disabled="extrasBusy"
                   @change="setAutoswitch({ enabled: $event.target.checked })"/>
            {{ autoswitch.enabled ? 'On' : 'Off' }}
          </label>
        </div>

        <div class="rules" v-if="autoswitch.rules && autoswitch.rules.length">
          <div class="rule" v-for="r in autoswitch.rules" :key="r.executable">
            <input type="checkbox" :checked="r.enabled" :disabled="extrasBusy"
                   :title="r.enabled ? 'Active' : 'Parked'"
                   @change="setAutoswitch({ toggle: [r.executable, $event.target.checked] })"/>
            <span class="exe">{{ r.executable }}</span>
            <select :value="r.profile" :disabled="extrasBusy"
                    @change="setAutoswitch({ set: [r.executable, $event.target.value] })">
              <option v-for="p in profiles" :key="p" :value="p">{{ p }}</option>
            </select>
            <button class="ghost" :disabled="extrasBusy"
                    @click="setAutoswitch({ remove: r.executable })">Remove</button>
          </div>
        </div>
        <div class="hint" v-else>No rules yet.</div>

        <!--
          No profile picker here. The rule takes the profile that is loaded
          right now, because that is how you were choosing it anyway: set the
          device up the way you want it for this game, then add the rule. The
          per-rule dropdown above is there to change it afterwards.
        -->
        <div class="inline addrule">
          <input type="text" v-model="newExe" :placeholder="foreground || 'something.exe'"/>
          <button class="ghost" :disabled="extrasBusy || !canAddRule" @click="addRule">
            Add as {{ profile || '…' }}
          </button>
        </div>

        <div class="hint">
          Adding uses the profile loaded now, so set the device up the way you
          want it and then add the rule. Matched on the executable name,
          because window titles change with what is open in them. Nothing
          reverts when you alt-tab away &mdash; leaving a game should not
          change how your music sounds.
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
          Use on all 4 channels &mdash; you only wear one pair
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
      // Fixed viewBox units. The SVG does not preserve aspect ratio, so these
      // are a drawing grid rather than a size -- nothing needs measuring.
      W: 1000,
      H: 400,
      profile: null,
      apo: null,
      buses: [],
      voicings: [],
      bandCentres: [],
      limit: 12,
      macroLimit: 10,
      selectedName: "Game",
      dragging: null,
      onMove: null,
      onUp: null,
      busy: false,
      error: null,
      status: null,
      searching: false,
      query: "",
      hits: [],
      searched: false,
      spatial: null,
      bypassed: false,
      extras: null,
      extrasBusy: false,
      extrasAllBuses: true,
      pluginLoaded: null,
      autoswitch: { enabled: false, rules: [] },
      foreground: null,
      newExe: "",
      levels: {},
      meterTimer: null,
      autoswitchTimer: null,
      spatialAllBuses: true,
      spatialBusy: false,
      spatialError: null,
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
    // Derived from the limit the daemon reports rather than hard-coded, so the
    // scale still reads correctly if that limit ever changes.
    interference() {
      return (this.extras && this.extras.interference) || [];
    },
    crossfeedPresets() {
      return (this.extras && this.extras.crossfeed_presets) || [];
    },
    plugins() {
      return (this.extras && this.extras.plugins) || [];
    },
    pluginGuidance() {
      return (this.extras && this.extras.plugin_guidance) || "";
    },
    profiles() {
      return (this.extras && this.extras.profiles) || [];
    },
    limits() {
      return (
        (this.extras && this.extras.crossfeed_limits) || {
          level_db: [1, 12],
          cutoff_hz: [300, 2000],
          delay_us: [0, 600],
        }
      );
    },
    /// What Add would use: whatever was typed, or failing that whatever is in
    /// front. The placeholder shows the latter, so an empty box is a choice
    /// rather than an omission.
    pendingExe() {
      return (this.newExe || this.foreground || "").trim();
    },
    canAddRule() {
      return Boolean(this.pendingExe && this.profile);
    },
    busExtras() {
      return (this.selected && this.selected.extras) || { crossfeed: null, plugin: null };
    },
    // Mirrors the server's arithmetic so the cost of crossfeed is visible
    // while dragging, rather than only after the round trip.
    crossfeedHeadroom() {
      const c = this.busExtras.crossfeed;
      if (!c) return 0;
      const g = Math.pow(10, -c.level_db / 20);
      return 20 * Math.log10(1 + g);
    },
    // Spatial state is per endpoint, so it follows the selected bus rather
    // than being a single setting for the tab.
    spatialBus() {
      if (!this.spatial || !this.spatial.buses) return null;
      return this.spatial.buses.find((b) => b.name === this.selectedName) || null;
    },
    gainGrid() {
      const l = this.limit;
      return [l, l / 2, 0, -l / 2, -l];
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
    this.loadSpatial();
    this.loadExtras();
    this.loadAutoswitch();
    // Meters poll while the tab is open. The daemon stops capturing five
    // seconds after the last read, so closing the tab stops the loopback
    // streams without anything having to say so.
    this.meterTimer = setInterval(() => this.pollMeters(), 100);
    this.autoswitchTimer = setInterval(() => this.loadAutoswitch(), 3000);
  },

  beforeUnmount() {
    if (this.searchTimer) clearTimeout(this.searchTimer);
    if (this.saveTimer) clearTimeout(this.saveTimer);
    if (this.meterTimer) clearInterval(this.meterTimer);
    if (this.autoswitchTimer) clearInterval(this.autoswitchTimer);
    this.releasePointer();
    // Leaving the tab while bypassed would leave the audio uncorrected with
    // nothing on screen to explain it.
    if (this.bypassed) this.setBypass(false);
  },

  methods: {
    async getJSON(url, opts) {
      const r = await fetch(url, opts);
      const body = await r.json().catch(() => ({ error: "Bad response from daemon" }));
      if (!r.ok || body.error) throw new Error(body.error || "HTTP " + r.status);
      return body;
    },

    // ---- geometry ----
    //
    // Two coordinate systems, deliberately. The curve and grid are drawn in
    // viewBox units and stretched to the box; the handles are placed in
    // percentages so they stay round. xPct/yPct are the source of truth and
    // xFor/yForGain are simply those scaled into viewBox units, so the dots
    // cannot drift away from the line they sit on.

    xPct(hz) {
      const lo = Math.log(F_MIN), hi = Math.log(F_MAX);
      return ((Math.log(hz) - lo) / (hi - lo)) * 100;
    },
    // 0% is +limit dB at the top, 100% is -limit at the bottom. The 4% inset
    // keeps a handle at either extreme fully inside the box.
    yPct(db) {
      const clamped = Math.max(-this.limit, Math.min(this.limit, db));
      return 50 - (clamped / this.limit) * 46;
    },
    gainForPct(pct) {
      return ((50 - pct) / 46) * this.limit;
    },

    xFor(hz) {
      return (this.xPct(hz) / 100) * this.W;
    },
    yForGain(db) {
      return (this.yPct(db) / 100) * this.H;
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

    /// Listeners go on the window rather than the handle, so a drag that runs
    /// off the top or bottom of the plot keeps tracking instead of sticking at
    /// wherever the pointer left the element.
    startDrag(index, event) {
      event.preventDefault();
      this.dragging = index;
      this.onMove = (e) => this.applyDrag(e);
      this.onUp = () => this.endDrag();
      window.addEventListener("pointermove", this.onMove);
      window.addEventListener("pointerup", this.onUp);
      window.addEventListener("pointercancel", this.onUp);
      this.applyDrag(event);
    },
    releasePointer() {
      if (!this.onMove) return;
      window.removeEventListener("pointermove", this.onMove);
      window.removeEventListener("pointerup", this.onUp);
      window.removeEventListener("pointercancel", this.onUp);
      this.onMove = null;
      this.onUp = null;
    },
    applyDrag(event) {
      const box = this.$refs.plot;
      if (!box || this.dragging === null) return;
      const rect = box.getBoundingClientRect();
      if (rect.height <= 0) return;
      const pct = ((event.clientY - rect.top) / rect.height) * 100;
      const gain = Math.max(-this.limit, Math.min(this.limit, this.gainForPct(pct)));
      this.selected.manual[this.dragging] = Math.round(gain * 2) / 2;
      this.queueSave();
    },
    endDrag() {
      if (this.dragging === null) return;
      this.dragging = null;
      this.releasePointer();
      this.queueSave(0);
    },
    nudge(index, delta) {
      const next = this.selected.manual[index] + delta;
      this.selected.manual[index] = Math.max(-this.limit, Math.min(this.limit, next));
      this.queueSave();
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
        this.status = `Applied to ${r.buses} channel(s) for the ${r.profile} profile.`;
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

    // ---- meters ----

    meterFor(bus) {
      return this.levels[bus] || { rms_dbfs: -90, peak_dbfs: -90, clipped: false };
    },
    /// Map dBFS to a bar width. Linear in dB rather than in amplitude,
    /// because that is how loudness reads: a linear-amplitude meter spends
    /// nine tenths of its travel in the top 20 dB and shows nothing useful.
    meterWidth(dbfs) {
      const floor = -60;
      const clamped = Math.max(floor, Math.min(0, dbfs));
      return ((clamped - floor) / -floor) * 100 + "%";
    },
    async pollMeters() {
      try {
        const r = await this.getJSON("/api/attune/extras/meters");
        this.levels = r.levels || {};
      } catch (e) {
        // Metering is a nicety. If it stops working the rest of the tab
        // should carry on without an error banner over it.
        this.levels = {};
      }
    },

    // ---- extras ----

    async loadExtras() {
      try {
        this.extras = await this.getJSON("/api/attune/extras/state");
        this.bypassed = this.extras.bypassed;
      } catch (e) {
        this.extras = null;
      }
    },

    async setBypass(on) {
      if (on === this.bypassed) return;
      this.bypassed = on;
      try {
        await this.getJSON("/api/attune/extras/bypass", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ on }),
        });
      } catch (e) {
        this.error = e.message;
        this.bypassed = !on;
      }
    },

    presetActive(preset) {
      const c = this.busExtras.crossfeed;
      if (!c) return false;
      const s = preset.settings;
      return (
        Math.abs(c.level_db - s.level_db) < 0.01 &&
        Math.abs(c.cutoff_hz - s.cutoff_hz) < 0.01 &&
        Math.abs(c.delay_us - s.delay_us) < 0.01
      );
    },

    async setCrossfeed(settings) {
      this.extrasBusy = true;
      try {
        await this.getJSON("/api/attune/extras/crossfeed", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            bus: this.selectedName,
            crossfeed: settings,
            all_buses: this.extrasAllBuses,
          }),
        });
        await this.load();
      } catch (e) {
        this.error = e.message;
      } finally {
        this.extrasBusy = false;
      }
    },

    tweakCrossfeed(field, value) {
      const next = { ...this.busExtras.crossfeed };
      next[field] = parseFloat(value);
      this.setCrossfeed(next);
    },

    async setPlugin(plugin) {
      this.extrasBusy = true;
      try {
        const r = await this.getJSON("/api/attune/extras/plugin", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            bus: this.selectedName,
            plugin,
            all_buses: this.extrasAllBuses,
          }),
        });
        // null means undetermined, which is not the same as "did not load".
        this.pluginLoaded = plugin ? r.loaded : null;
        await this.load();
      } catch (e) {
        this.error = e.message;
      } finally {
        this.extrasBusy = false;
      }
    },

    // ---- per-game profiles ----

    async loadAutoswitch() {
      try {
        const r = await this.getJSON("/api/attune/extras/autoswitch");
        this.autoswitch = r.rules;
        this.foreground = r.foreground;
      } catch (e) {
        // Leave whatever was last known rather than blanking the card.
      }
    },

    async setAutoswitch(change) {
      this.extrasBusy = true;
      try {
        const r = await this.getJSON("/api/attune/extras/autoswitch", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(change),
        });
        this.autoswitch = r.rules;
      } catch (e) {
        this.error = e.message;
      } finally {
        this.extrasBusy = false;
      }
    },

    addRule() {
      if (!this.canAddRule) return;
      this.setAutoswitch({ set: [this.pendingExe, this.profile] });
      this.newExe = "";
    },

    // ---- spatial audio ----

    async loadSpatial() {
      try {
        this.spatial = await this.getJSON("/api/attune/spatial/state");
      } catch (e) {
        // Not fatal. The rest of the tab works without it, so this reports
        // itself in its own card rather than taking over the page.
        this.spatial = { supported: false, buses: [] };
        this.spatialError = e.message;
      }
    },

    async setSpatial(format) {
      if (format.subtype === this.spatialBus.active && !this.spatialAllBuses) return;
      this.spatialBusy = true;
      this.spatialError = null;
      try {
        const r = await this.getJSON("/api/attune/spatial/set", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            bus: this.selectedName,
            subtype: format.subtype,
            all_buses: this.spatialAllBuses,
          }),
        });
        // Partial success is normal -- four endpoints, any of which may be
        // absent -- so say which ones did not take rather than nothing.
        if (r.failed && r.failed.length) this.spatialError = r.failed.join(" · ");
      } catch (e) {
        this.spatialError = e.message;
      } finally {
        await this.loadSpatial();
        this.spatialBusy = false;
      }
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
.hptab { padding: 22px 30px 50px; color: #fff; text-align: left; }

.topbar { display: flex; justify-content: space-between; align-items: center;
          gap: 16px; margin-bottom: 14px; flex-wrap: wrap; }
.bustabs { display: flex; gap: 2px; }
.bustab { position: relative; background: none; color: #8d9591; border: 0;
          border-bottom: 2px solid transparent; padding: 8px 18px 12px;
          font-family: inherit; font-size: 14px; cursor: pointer; }
.bustab.on { color: #fff; border-bottom-color: #59b1b6; }
.bustab .dot { position: absolute; top: 6px; right: 6px; width: 5px; height: 5px;
               border-radius: 50%; background: #59b1b6; }

.topright { display: flex; align-items: center; gap: 8px; }
.compare { min-width: 0; user-select: none; touch-action: none; }
.compare.held { background-color: #d9a441; color: #1b1f1e; font-weight: 600; }

/* A bar under each bus name. Linear in dB, because a linear-amplitude meter
   spends nine tenths of its travel in the top 20 dB. */
.meter { position: absolute; left: 10px; right: 10px; bottom: 2px; height: 3px;
         background: #1b1f1e; border-radius: 2px; overflow: hidden; }
.meter .rms { position: absolute; left: 0; top: 0; bottom: 0; background: #59b1b6;
              transition: width .08s linear; }
.meter .peak { position: absolute; top: 0; bottom: 0; width: 2px;
               background: #b4bcb8; transition: left .08s linear; }
.meter.clip { background: #e0655b; }
.meter.clip .rms { background: #e0655b; }

.rules { display: flex; flex-direction: column; gap: 4px; margin-bottom: 10px; }
.rule { display: flex; align-items: center; gap: 8px; background: #252927;
        padding: 6px 10px; }
.rule .exe { flex: 1; font-size: 12px; font-variant-numeric: tabular-nums; }
.rule select { width: auto; min-width: 150px; }
.rule input[type="checkbox"] { width: auto; }
.addrule { gap: 6px; }
.addrule input[type="text"] { flex: 1; }
.addrule select { width: auto; min-width: 150px; }
.check.switch { margin-left: auto; }
code { background: #1b1f1e; padding: 1px 5px; border-radius: 3px;
       font-family: inherit; font-size: 11px; }
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

/* 42px of y-axis labels plus the 8px plotwrap gap, so the region headers line
   up with the plot underneath them rather than with the card edge. */
.regions { display: flex; gap: 2px; margin: 0 0 6px 50px; }
.region { background: #252927; color: #8d9591; font-size: 10px;
          letter-spacing: .6px; text-align: center; padding: 5px 0; }

.plotwrap { display: flex; gap: 8px; }
.ylabels { position: relative; width: 42px; height: 220px; flex: 0 0 auto; }
.ylabels span { position: absolute; right: 0; transform: translateY(-50%);
                color: #6b736f; font-size: 10px; white-space: nowrap;
                font-variant-numeric: tabular-nums; }
.plotbox { position: relative; width: 100%; height: 220px; background: #252927;
           touch-action: none; }
.plot { display: block; width: 100%; height: 100%; }

/* Handles are HTML, not SVG, so preserveAspectRatio="none" on the plot cannot
   squash them into ovals. Positioned in percent, centred on their own point.
   min-width/max-width are stated because goxlr-ui ships an unscoped
   `.tab button { min-width: 150px }`, which a scoped `width: 16px` loses to.
   This component no longer uses the class `tab`, so that rule cannot reach it
   any more -- these are here so the next such collision cannot either. */
.handle { position: absolute; width: 16px; height: 16px;
          min-width: 0; max-width: none; padding: 0;
          margin: 0; border-radius: 50%; border: 2px solid #1b1f1e;
          transform: translate(-50%, -50%); cursor: ns-resize;
          touch-action: none; transition: width .1s, height .1s; }
.handle:hover, .handle:focus-visible { width: 20px; height: 20px; outline: none; }
.handle.dragging { width: 22px; height: 22px; box-shadow: 0 0 0 4px rgba(89,177,182,.25); }

.plotcol { flex: 1; display: flex; flex-direction: column; min-width: 0; }
.xlabels { position: relative; height: 16px; margin-top: 3px; }
.xlabels span { position: absolute; transform: translateX(-50%);
                color: #6b736f; font-size: 10px; }
.plothint { margin-top: 6px; }

.formats { display: flex; flex-direction: column; gap: 3px; margin-bottom: 10px; }
.format { display: flex; flex-direction: column; align-items: flex-start; gap: 2px;
          text-align: left; background: #252927; padding: 8px 11px;
          border-left: 3px solid transparent; }
.format:hover:not(:disabled) { background: #343b38; }
.format.on { border-left-color: #59b1b6; background: #2f3835; }
.format.off { opacity: .45; cursor: not-allowed; }
.fname { font-size: 12px; display: flex; align-items: center; gap: 7px; }
.ftag { font-size: 9px; letter-spacing: .5px; text-transform: uppercase;
        color: #d9a441; border: 1px solid #5c4a24; border-radius: 8px;
        padding: 1px 6px; }
.fnote { font-size: 10px; color: #8d9591; line-height: 1.4; }

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
/* Louder than "missing" on purpose: a detached endpoint means the curves are
   drawn, saved and doing nothing, which reads as working until you listen. */
.apo.wiped { border-left-color: #e0655b; color: #e8b3ae; }
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
