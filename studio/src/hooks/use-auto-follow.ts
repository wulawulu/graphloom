import { useCallback, useEffect, useRef, useState } from "react"

/** Distance from the bottom that still counts as reading the latest output. */
export const FOLLOW_THRESHOLD_PX = 96

const PROGRAMMATIC_TARGET_EPSILON_PX = 2
const SMOOTH_IDLE_TIMEOUT_MS = 150
const SMOOTH_START_TIMEOUT_MS = 1_000

interface AutoFollowOptions {
  resetKey: string | null
  threshold?: number
}

interface AutoFollowController {
  viewportRef: React.RefObject<HTMLDivElement | null>
  contentRef: React.RefObject<HTMLDivElement | null>
  following: boolean
  pause: () => void
  resume: () => void
  scrollToLatest: () => void
}

interface SmoothOperation {
  generation: number
  target: number
  contentChanged: boolean
}

/** Returns the viewport's non-negative distance from its latest scroll position. */
export function distanceFromBottom(viewport: Pick<HTMLElement, "clientHeight" | "scrollHeight" | "scrollTop">): number {
  return Math.max(0, viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight)
}

/** Reports whether a viewport remains within the latest-content threshold. */
export function isNearBottom(
  viewport: Pick<HTMLElement, "clientHeight" | "scrollHeight" | "scrollTop">,
  threshold = FOLLOW_THRESHOLD_PX,
): boolean {
  return distanceFromBottom(viewport) <= threshold
}

function latestScrollTop(viewport: Pick<HTMLElement, "clientHeight" | "scrollHeight">): number {
  return Math.max(0, viewport.scrollHeight - viewport.clientHeight)
}

/** Keeps a scroll viewport on newly laid-out content until the reader scrolls away. */
export function useAutoFollow({ resetKey, threshold = FOLLOW_THRESHOLD_PX }: AutoFollowOptions): AutoFollowController {
  const viewportRef = useRef<HTMLDivElement>(null)
  const contentRef = useRef<HTMLDivElement>(null)
  const shouldFollowRef = useRef(true)
  const programmaticTargetRef = useRef<number | null>(null)
  const smoothOperationRef = useRef<SmoothOperation | null>(null)
  const operationGenerationRef = useRef(0)
  const scrollFrameRef = useRef<number | null>(null)
  const smoothTimerRef = useRef<number | null>(null)
  const [following, setFollowing] = useState(true)

  const cancelScrollFrame = useCallback((): void => {
    if (scrollFrameRef.current === null) return
    cancelAnimationFrame(scrollFrameRef.current)
    scrollFrameRef.current = null
  }, [])

  const clearSmoothTimer = useCallback((): void => {
    if (smoothTimerRef.current === null) return
    window.clearTimeout(smoothTimerRef.current)
    smoothTimerRef.current = null
  }, [])

  const setFollowState = useCallback((next: boolean): void => {
    shouldFollowRef.current = next
    setFollowing((current) => current === next ? current : next)
    if (!next) cancelScrollFrame()
  }, [cancelScrollFrame])

  const performAutoScroll = useCallback((): void => {
    const viewport = viewportRef.current
    if (viewport === null) return
    programmaticTargetRef.current = latestScrollTop(viewport)
    viewport.scrollTo({ top: viewport.scrollHeight, behavior: "auto" })
  }, [])

  const scheduleScrollToLatest = useCallback((): void => {
    if (!shouldFollowRef.current || scrollFrameRef.current !== null || smoothOperationRef.current !== null) return
    scrollFrameRef.current = requestAnimationFrame(() => {
      scrollFrameRef.current = null
      if (shouldFollowRef.current && smoothOperationRef.current === null) performAutoScroll()
    })
  }, [performAutoScroll])

  const finishSmoothScroll = useCallback((generation: number): void => {
    const operation = smoothOperationRef.current
    if (operation === null || operation.generation !== generation) return
    const viewport = viewportRef.current
    smoothOperationRef.current = null
    clearSmoothTimer()
    if (viewport === null || !shouldFollowRef.current) return

    const expectedTarget = Math.min(operation.target, latestScrollTop(viewport))
    const reachedTarget = Math.abs(viewport.scrollTop - expectedTarget) <= PROGRAMMATIC_TARGET_EPSILON_PX
    if (!reachedTarget) {
      setFollowState(false)
      return
    }
    if (operation.contentChanged) scheduleScrollToLatest()
    else setFollowState(isNearBottom(viewport, threshold))
  }, [clearSmoothTimer, scheduleScrollToLatest, setFollowState, threshold])

  const armSmoothTimer = useCallback((generation: number, delay: number): void => {
    clearSmoothTimer()
    smoothTimerRef.current = window.setTimeout(() => {
      smoothTimerRef.current = null
      finishSmoothScroll(generation)
    }, delay)
  }, [clearSmoothTimer, finishSmoothScroll])

  const stopSmoothScroll = useCallback((): void => {
    const operation = smoothOperationRef.current
    operationGenerationRef.current += 1
    smoothOperationRef.current = null
    clearSmoothTimer()
    if (operation === null) return

    const viewport = viewportRef.current
    if (viewport === null) return
    programmaticTargetRef.current = viewport.scrollTop
    viewport.scrollTo({ top: viewport.scrollTop, behavior: "auto" })
  }, [clearSmoothTimer])

  const pause = useCallback((): void => {
    setFollowState(false)
    stopSmoothScroll()
  }, [setFollowState, stopSmoothScroll])

  const resume = useCallback((): void => setFollowState(true), [setFollowState])

  const scrollToLatest = useCallback((): void => {
    const viewport = viewportRef.current
    if (viewport === null) return
    setFollowState(true)
    stopSmoothScroll()

    const reducedMotion = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false
    if (reducedMotion) {
      performAutoScroll()
      return
    }

    const generation = operationGenerationRef.current + 1
    operationGenerationRef.current = generation
    smoothOperationRef.current = {
      generation,
      target: latestScrollTop(viewport),
      contentChanged: false,
    }
    armSmoothTimer(generation, SMOOTH_START_TIMEOUT_MS)
    viewport.scrollTo({ top: viewport.scrollHeight, behavior: "smooth" })
  }, [armSmoothTimer, performAutoScroll, setFollowState, stopSmoothScroll])

  useEffect(() => {
    const viewport = viewportRef.current
    const content = contentRef.current
    if (viewport === null || content === null) return undefined

    let active = true
    operationGenerationRef.current += 1
    smoothOperationRef.current = null
    programmaticTargetRef.current = null
    clearSmoothTimer()
    cancelScrollFrame()
    setFollowState(true)

    const syncFollowFromScroll = (): void => {
      const target = programmaticTargetRef.current
      if (target !== null) {
        programmaticTargetRef.current = null
        if (Math.abs(viewport.scrollTop - target) <= PROGRAMMATIC_TARGET_EPSILON_PX) return
      }

      const smoothOperation = smoothOperationRef.current
      if (smoothOperation !== null) {
        armSmoothTimer(smoothOperation.generation, SMOOTH_IDLE_TIMEOUT_MS)
        return
      }
      setFollowState(isNearBottom(viewport, threshold))
    }
    const finishCurrentSmoothScroll = (): void => {
      const operation = smoothOperationRef.current
      if (operation !== null) finishSmoothScroll(operation.generation)
    }
    const observer = new ResizeObserver(() => {
      if (!active || !shouldFollowRef.current) return
      const smoothOperation = smoothOperationRef.current
      if (smoothOperation !== null) {
        smoothOperation.contentChanged = true
        return
      }
      scheduleScrollToLatest()
    })

    viewport.addEventListener("scroll", syncFollowFromScroll, { passive: true })
    viewport.addEventListener("scrollend", finishCurrentSmoothScroll)
    observer.observe(content)
    scheduleScrollToLatest()

    return () => {
      active = false
      observer.disconnect()
      viewport.removeEventListener("scroll", syncFollowFromScroll)
      viewport.removeEventListener("scrollend", finishCurrentSmoothScroll)
      cancelScrollFrame()
      stopSmoothScroll()
      programmaticTargetRef.current = null
    }
  }, [armSmoothTimer, cancelScrollFrame, clearSmoothTimer, finishSmoothScroll, resetKey, scheduleScrollToLatest, setFollowState, stopSmoothScroll, threshold])

  return { viewportRef, contentRef, following, pause, resume, scrollToLatest }
}
