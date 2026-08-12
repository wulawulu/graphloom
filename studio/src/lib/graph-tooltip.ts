export function clampTooltipPosition(x: number, y: number, viewportWidth: number, viewportHeight: number, width: number, height: number): { x: number; y: number } {
  const margin = 8
  return {
    x: Math.max(margin, Math.min(x + 12, viewportWidth - width - margin)),
    y: Math.max(margin, Math.min(y + 12, viewportHeight - height - margin)),
  }
}
