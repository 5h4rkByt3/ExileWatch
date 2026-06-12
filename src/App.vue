<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import PriceCheck from './components/PriceCheck.vue'
import SetupScreen from './components/SetupScreen.vue'

const locked = ref(false)
const showSetup = ref(false)
const unlisteners: UnlistenFn[] = []

function handleInteract() { locked.value = true }

function handleClose() {
  locked.value = false
  invoke('hide_overlay')
}

async function handleSetupDone() {
  await invoke('complete_setup')
  showSetup.value = false
  locked.value = false
  invoke('hide_overlay')
}

onMounted(async () => {
  const needsSetup = await invoke<boolean>('is_setup_needed')
  if (needsSetup) {
    showSetup.value = true
    locked.value = true
  }

  unlisteners.push(await listen('alt-released', () => {
    if (!locked.value) invoke('hide_overlay')
  }))

  unlisteners.push(await listen('overlay-shown', () => {
    locked.value = true
  }))

  unlisteners.push(await listen('search-failed', () => {
    if (!showSetup.value) {
      locked.value = false
      invoke('hide_overlay')
    }
  }))

  unlisteners.push(await listen('escape-pressed', () => {
    if (!showSetup.value) {
      locked.value = false
      invoke('hide_overlay')
    }
  }))
})

onUnmounted(() => { unlisteners.forEach(fn => fn()) })
</script>

<template>
  <SetupScreen v-if="showSetup" @done="handleSetupDone" />
  <PriceCheck v-else @interact="handleInteract" @close="handleClose" />
</template>
