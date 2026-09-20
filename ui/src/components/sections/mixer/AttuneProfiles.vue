<!--
  Attune profiles: one switch for the mic and the headphones together.

  A profile stores only what the GoXLR cannot -- the per-bus headphone curves --
  and references a GoXLR mic profile by name. The device stays authoritative for
  the mic chain; duplicating it here would create two stores that drift.
-->
<template>
  <GroupContainer title="Profiles" :side-padding="'16px'">
    <div class="col">

      <div class="bad" v-if="error">{{ error }}</div>

      <div class="plist" v-if="profiles.length">
        <div class="prow" v-for="p in profiles" :key="p.name" :class="{ on: p.name === active }">
          <button class="pname" @click="applyProfile(p)" :disabled="busy" :title="'Apply ' + p.name">
            <span class="pn">{{ p.name }}</span>
            <span class="pd">{{ summary(p) }}</span>
          </button>
          <button class="del" @click="removeProfile(p)" :disabled="busy" title="Delete">&times;</button>
        </div>
      </div>
      <div class="hint" v-else>
        No profiles yet. Set your voicings and corrections on the Headphones
        panel, then save them here under a name.
      </div>

      <div class="save">
        <input v-model="newName" placeholder="Profile name" :disabled="busy"/>
        <select v-model="micProfile" :disabled="busy">
          <option value="">no mic profile</option>
          <option v-for="m in micProfiles" :key="m" :value="m">mic: {{ m }}</option>
        </select>
        <button class="accent" @click="saveProfile" :disabled="busy || !newName.trim()">
          Save current
        </button>
      </div>

      <div class="hint">
        Saving captures the headphone curves as they are now. The mic profile is
        referenced by name, so the GoXLR stays in charge of the mic chain.
      </div>

      <div class="result" v-if="result.length">
        <div class="good" v-for="(r, i) in result" :key="'a' + i">{{ r }}</div>
      </div>
      <div class="result" v-if="failures.length">
        <div class="bad" v-for="(f, i) in failures" :key="'f' + i">{{ f }}</div>
      </div>

    </div>
  </GroupContainer>
</template>

<script>
import GroupContainer from "@/components/containers/GroupContainer.vue";

export default {
  name: "AttuneProfiles",
  components: { GroupContainer },

  data() {
    return {
      profiles: [],
      active: null,
      micProfiles: [],
      newName: "",
      micProfile: "",
      busy: false,
      error: null,
      result: [],
      failures: [],
    };
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

    summary(p) {
      const parts = Object.entries(p.buses || {})
        .filter(([, v]) => v && v !== "neutral")
        .map(([k, v]) => k + ":" + v);
      if (p.mic_profile) parts.unshift("mic:" + p.mic_profile);
      return parts.length ? parts.join("  ") : "neutral everywhere";
    },

    async load() {
      try {
        const s = await this.getJSON("/api/attune/profiles");
        this.profiles = s.profiles;
        this.active = s.active;
        this.micProfiles = s.mic_profiles || [];
        this.error = null;
      } catch (e) {
        this.error = e.message;
      }
    },

    async saveProfile() {
      this.busy = true;
      this.error = null;
      this.result = [];
      this.failures = [];
      try {
        await this.getJSON("/api/attune/profiles/save", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            name: this.newName.trim(),
            mic_profile: this.micProfile || null,
          }),
        });
        this.newName = "";
        await this.load();
      } catch (e) {
        this.error = e.message;
      } finally {
        this.busy = false;
      }
    },

    async applyProfile(p) {
      this.busy = true;
      this.error = null;
      this.result = [];
      this.failures = [];
      try {
        const r = await this.getJSON("/api/attune/profiles/apply", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ name: p.name }),
        });
        // Partial success is reported as such: one half landing and the other
        // failing is a real state, and calling it "applied" would hide it.
        this.result = r.applied || [];
        this.failures = r.failed || [];
        await this.load();
        this.$emit("profile-applied");
      } catch (e) {
        this.error = e.message;
      } finally {
        this.busy = false;
      }
    },

    async removeProfile(p) {
      this.busy = true;
      this.error = null;
      try {
        await this.getJSON("/api/attune/profiles/delete", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ name: p.name }),
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
.col { display: flex; flex-direction: column; gap: 9px; width: 280px;
       color: #fff; font-size: 14px; text-align: left; }

.plist { display: flex; flex-direction: column; gap: 3px;
         max-height: 190px; overflow-y: auto; }
.plist::-webkit-scrollbar { width: 5px; }
.plist::-webkit-scrollbar-thumb { background: #3b413f; border-radius: 3px; }

.prow { display: flex; gap: 3px; }
.prow.on .pname { border-left: 3px solid #59b1b6; }

.pname { flex: 1; display: flex; flex-direction: column; align-items: flex-start; gap: 1px;
         text-align: left; background: #252927; padding: 7px 9px;
         border-left: 3px solid transparent; }
.pname:hover:not(:disabled) { background: #313734; }
.pn { font-size: 13px; color: #fff; }
.pd { font-size: 10px; color: #8d9591; }

.del { background: #252927; color: #8d9591; padding: 0 10px; font-size: 16px; line-height: 1; }
.del:hover:not(:disabled) { background: #3b2422; color: #e0655b; }

.save { display: flex; flex-direction: column; gap: 5px;
        border-top: 1px solid #3b413f; padding-top: 8px; }

input, select {
  background-color: #252927; color: #fff;
  border: 1px solid #3b413f; border-radius: 3px;
  padding: 6px 8px; font-family: inherit; font-size: 12px; width: 100%;
}

button {
  background-color: #3b413f; color: #fff; border: 0; border-radius: 3px;
  padding: 7px 11px; font-family: inherit; font-size: 12px; cursor: pointer;
}
button.accent { background-color: #59b1b6; color: #0d1817; font-weight: 600; }
button:disabled { opacity: .45; cursor: not-allowed; }

.hint { color: #8d9591; font-size: 11px; line-height: 1.45; }

.result { display: flex; flex-direction: column; gap: 4px; }
.good { color: #6fc79b; font-size: 12px; }
.bad { color: #e0655b; font-size: 12px; line-height: 1.45;
       border-left: 3px solid #e0655b; background: #252927;
       padding: 7px 9px; border-radius: 0 3px 3px 0; }
</style>
