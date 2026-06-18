<script setup lang="ts">
import { state, closeDialog } from '../composables/useDialog'
</script>

<template>
  <Teleport to="body">
    <div
      v-if="state.visible"
      class="fixed inset-0 z-50 flex items-center justify-center"
    >
      <div class="absolute inset-0 bg-black/50" @click="closeDialog" />
      <div class="relative bg-white rounded-xl shadow-2xl w-full max-w-sm mx-4 p-6">
        <div class="flex items-start gap-4">
          <div
            class="w-10 h-10 rounded-full flex items-center justify-center text-lg font-bold shrink-0"
            :class="{
              'bg-blue-100 text-blue-600': state.type === 'confirm' || state.type === 'info',
              'bg-green-100 text-green-600': state.type === 'success',
              'bg-red-100 text-red-600': state.type === 'error',
              'bg-yellow-100 text-yellow-600': state.type === 'warning',
            }"
          >
            <span v-if="state.type === 'confirm'">?</span>
            <span v-else-if="state.type === 'success'">✓</span>
            <span v-else-if="state.type === 'error'">✕</span>
            <span v-else-if="state.type === 'warning'">⚠</span>
            <span v-else>i</span>
          </div>
          <div class="flex-1 min-w-0">
            <h3 v-if="state.title" class="text-lg font-semibold text-gray-900 mb-2">
              {{ state.title }}
            </h3>
            <p class="text-sm text-gray-600 break-words">{{ state.message }}</p>
          </div>
        </div>
        <div class="flex justify-end gap-3 mt-6">
          <button
            v-if="state.type === 'confirm'"
            @click="closeDialog"
            class="px-4 py-2 text-sm font-medium text-gray-700 bg-gray-100 rounded-lg hover:bg-gray-200"
          >
            {{ state.cancelText }}
          </button>
          <button
            @click="state.visible = false; state.onConfirm?.()"
            class="px-4 py-2 text-sm font-medium text-white rounded-lg"
            :class="{
              'bg-blue-600 hover:bg-blue-700': state.type === 'confirm' || state.type === 'info',
              'bg-green-600 hover:bg-green-700': state.type === 'success',
              'bg-red-600 hover:bg-red-700': state.type === 'error',
              'bg-yellow-600 hover:bg-yellow-700': state.type === 'warning',
            }"
          >
            {{ state.confirmText }}
          </button>
        </div>
      </div>
    </div>
  </Teleport>
</template>
