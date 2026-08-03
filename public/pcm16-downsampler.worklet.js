class Pcm16Downsampler extends AudioWorkletProcessor {
  constructor() {
    super()
    this.targetRate = 16000
    this.frameSamples = 1600
    this.buffer = new Int16Array(this.frameSamples)
    this.write = 0
    this.sourceCursor = 0
    this.step = sampleRate / this.targetRate
  }

  process(inputs) {
    const input = inputs[0]
    if (!input || !input[0]) return true
    const channel = input[0]
    for (let i = 0; i < channel.length; i += 1) {
      this.sourceCursor += 1
      if (this.sourceCursor >= this.step) {
        this.sourceCursor -= this.step
        const sample = Math.max(-1, Math.min(1, channel[i]))
        this.buffer[this.write++] = sample < 0 ? sample * 0x8000 : sample * 0x7fff
        if (this.write >= this.frameSamples) {
          this.port.postMessage(this.buffer.slice(0))
          this.write = 0
        }
      }
    }
    return true
  }
}

registerProcessor("pcm16-downsampler", Pcm16Downsampler)
