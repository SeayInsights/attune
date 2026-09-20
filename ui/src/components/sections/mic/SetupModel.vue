<template>
  <!-- Build the Modal -->
  <CenteredContainer>
    <ContentContainer>
      <RadioSelection ref="selection" :title="$t('message.microphone.setup.type')" group="mic_type" :options="getMicrophoneOptions()" :selected="getActiveMicType()" @selection-changed="handleButtonPress" />

      <Slider :title="$t('message.microphone.setup.gain')" :slider-min-value=0 :slider-max-value=72 :text-suffix="$t('message.suffixes.decibels')"
              :slider-value=getGainValue() :store-path="getStorePath()" @value-changed="setGain" />

      <AudioMeter :active="polling" />

      <!--
        Attune sits in the setup flow because gain is the first thing it
        corrects, and gain is what this modal is for. Measuring it belongs
        next to setting it by hand, not on another screen.
      -->
      <AttuneTuner ref="attune" />
    </ContentContainer>
  </CenteredContainer>
</template>

<script>
import Slider from "@/components/slider/Slider.vue";
import {store} from "@/store";
import {websocket} from "@/util/sockets";
import RadioSelection from "@/components/lists/RadioSelection.vue";
import ContentContainer from "@/components/containers/ContentContainer.vue";
import CenteredContainer from "@/components/containers/CenteredContainer.vue";
import {isDeviceMini} from "@/util/util";
import AudioMeter from "@/components/sections/mic/AudioMeter.vue";
import AttuneTuner from "@/components/sections/mic/AttuneTuner.vue";

export default {
  name: "SetupModel",
  components: {AttuneTuner, AudioMeter, CenteredContainer, ContentContainer, RadioSelection, Slider},
  data: function() {
    return {
      polling: false,
      current_value: -72,
    }
  },

  methods: {
    getMicrophoneOptions() {
      let voltage = "48";
      if (isDeviceMini()) {
        voltage = "24";
      }

      return [
        {id: "Dynamic", label: this.$t('message.microphone.setup.xlr')},
        {id: "Condenser", label: this.$t('message.microphone.setup.phantom', {voltage: voltage})},
        {id: "Jack", label: this.$t('message.microphone.setup.jack')}
      ];
    },

    getActiveMicType() {
      return store.getActiveDevice().mic_status.mic_type
    },

    getGainValue() {
      return store.getActiveDevice().mic_status.mic_gains[store.getActiveDevice().mic_status.mic_type];
    },

    setGain(id, value) {
      let command = {
        "SetMicrophoneGain": [
          store.getActiveDevice().mic_status.mic_type,
          value
        ]
      };
      websocket.send_command(store.getActiveSerial(), command);
      store.getActiveDevice().mic_status.mic_gains[store.getActiveDevice().mic_status.mic_type] = value;
    },

    handleButtonPress(id) {
      let command = {
        "SetMicrophoneType": id
      }

      websocket.send_command(store.getActiveSerial(), command);
    },

    getStorePath() {
      return "/mixers/" + store.getActiveSerial() + "/mic_status/mic_gains/" + store.getActiveDevice().mic_status.mic_type;
    },

    focus() {
      let activeType = store.getActiveDevice().mic_status.mic_type;
      let button = this.$refs.selection.getButtonByRef(activeType);
      button.focus();
    },

    opened() {
      this.polling = true;
      this.$refs.attune?.refresh();
    },
    closed() {
      this.polling = false;
      // Closing the modal does not unmount it, so the countdown would keep
      // ticking against a recording nobody can see.
      this.$refs.attune?.stopDisplay();
    }
  },
}
</script>

<style scoped>

</style>
