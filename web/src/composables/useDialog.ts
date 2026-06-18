import { ref } from 'vue'

export type DialogType = 'confirm' | 'success' | 'error' | 'warning' | 'info'

export interface DialogState {
  visible: boolean
  title: string
  message: string
  type: DialogType
  confirmText: string
  cancelText: string
  onConfirm: (() => void) | null
  onCancel: (() => void) | null
}

export const state = ref<DialogState>({
  visible: false,
  title: '',
  message: '',
  type: 'confirm',
  confirmText: '确定',
  cancelText: '取消',
  onConfirm: null,
  onCancel: null,
})

export function showDialog(opts: {
  title?: string
  message: string
  type?: DialogType
  confirmText?: string
  cancelText?: string
}) {
  state.value.title = opts.title || ''
  state.value.message = opts.message
  state.value.type = opts.type || 'confirm'
  state.value.confirmText = opts.confirmText || '确定'
  state.value.cancelText = opts.cancelText || '取消'
  state.value.visible = true
}

export function closeDialog() {
  state.value.visible = false
  state.value.onCancel?.()
}

export function useDialog() {
  function confirm(message: string, title?: string): Promise<boolean | null> {
    return new Promise((resolve) => {
      state.value.onConfirm = () => {
        state.value.visible = false
        resolve(true)
      }
      state.value.onCancel = () => {
        state.value.visible = false
        resolve(false)
      }
      showDialog({ message, title, type: 'confirm' })
    })
  }

  function alert(message: string, title?: string, type: DialogType = 'info') {
    state.value.onConfirm = () => {
      state.value.visible = false
    }
    showDialog({ message, title, type, confirmText: '知道了' })
  }

  function error(message: string, title?: string) {
    alert(message, title || '错误', 'error')
  }

  function success(message: string, title?: string) {
    alert(message, title || '成功', 'success')
  }

  return { confirm, alert, error, success }
}
