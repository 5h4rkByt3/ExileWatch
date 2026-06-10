<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import PriceCheck from './components/PriceCheck.vue'

const locked = ref(false)
const unlisteners: UnlistenFn[] = []

function handleInteract() {
  locked.value = true
}

function handleClose() {
  locked.value = false
  invoke('hide_overlay')
}

onMounted(async () => {
  // Fired by evdev when Alt key is released globally
  unlisteners.push(await listen('alt-released', () => {
    if (!locked.value) invoke('hide_overlay')
  }))

  // Lock when freshly shown so Alt-release doesn't immediately close it
  unlisteners.push(await listen('overlay-shown', () => {
    locked.value = true
  }))

  // No item found or not a PoE item — hide overlay automatically
  unlisteners.push(await listen('search-failed', () => {
    locked.value = false
    invoke('hide_overlay')
  }))

  // Escape key pressed globally (evdev) while overlay is visible
  unlisteners.push(await listen('escape-pressed', () => {
    locked.value = false
    invoke('hide_overlay')
  }))
})

onUnmounted(() => {
  unlisteners.forEach(fn => fn())
})
</script>

<template>
  <PriceCheck @interact="handleInteract" @close="handleClose" />
</template>
