export function clampTooltipPosition(x: number, y: number, viewportWidth: number, viewportHeight: number): { x: number; y: number } {
  const width = 240
  const height = 88
  const margin = 8
  return {
    x: Math.max(margin, Math.min(x + 12, viewportWidth - width - margin)),
    y: Math.max(margin, Math.min(y + 12, viewportHeight - height - margin)),
  }
}
