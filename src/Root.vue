<script setup lang="ts">
import { onMounted } from "vue";
import MainShell from "./components/MainShell.vue";
import StickyNote from "./components/StickyNote.vue";
import { initializeTheme } from "./theme";

onMounted(() => void initializeTheme());

const params = new URLSearchParams(window.location.search);
const view = params.get("view");
const itemId = params.get("id");
</script>

<template>
  <StickyNote v-if="view === 'sticky' && itemId" :item-id="itemId" />
  <MainShell v-else />
</template>
