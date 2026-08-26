import { useCallback, useEffect, useRef, useState } from "react"

/** Distance from the bottom that still counts as reading the latest output. */
export const FOLLOW_THRESHOLD_PX = 96

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

/** Keeps a scroll viewport on newly laid-out content until the reader scrolls away. */
export function useAutoFollow({ resetKey, threshold = FOLLOW_THRESHOLD_PX }: AutoFollowOptions): AutoFollowController {
  const viewportRef = useRef<HTMLDivElement>(null)
  const contentRef = useRef<HTMLDivElement>(null)
  const shouldFollowRef = useRef(true)
  const programmaticScrollRef = useRef(false)
  const scrollFrameRef = useRef<number | null>(null)
  const guardFrameRef = useRef<number | null>(null)
  const smoothGuardTimerRef = useRef<number | null>(null)
  const [following, setFollowing] = useState(true)

  const cancelScrollFrame = useCallback((): void => {
    if (scrollFrameRef.current === null) return
    cancelAnimationFrame(scrollFrameRef.current)
    scrollFrameRef.current = null
  }, [])

  const clearProgrammaticGuard = useCallback((): void => {
    programmaticScrollRef.current = false
    if (guardFrameRef.current !== null) {
      cancelAnimationFrame(guardFrameRef.current)
      guardFrameRef.current = null
    }
    if (smoothGuardTimerRef.current !== null) {
      window.clearTimeout(smoothGuardTimerRef.current)
      smoothGuardTimerRef.current = null
    }
  }, [])

  const setFollowState = useCallback((next: boolean): void => {
    shouldFollowRef.current = next
    setFollowing((current) => current === next ? current : next)
    if (!next) cancelScrollFrame()
  }, [cancelScrollFrame])

  const releaseAutoScrollGuard = useCallback((): void => {
    if (guardFrameRef.current !== null) cancelAnimationFrame(guardFrameRef.current)
    guardFrameRef.current = requestAnimationFrame(() => {
      guardFrameRef.current = requestAnimationFrame(() => {
        guardFrameRef.current = null
        programmaticScrollRef.current = false
      })
    })
  }, [])

  const scrollLatest = useCallback((behavior: ScrollBehavior): void => {
    const viewport = viewportRef.current
    if (viewport === null) return

    programmaticScrollRef.current = true
    viewport.scrollTo({ top: viewport.scrollHeight, behavior })
    if (behavior === "smooth") {
      if (smoothGuardTimerRef.current !== null) window.clearTimeout(smoothGuardTimerRef.current)
      smoothGuardTimerRef.current = window.setTimeout(() => {
        smoothGuardTimerRef.current = null
        programmaticScrollRef.current = false
        const currentViewport = viewportRef.current
        if (currentViewport !== null) setFollowState(isNearBottom(currentViewport, threshold))
      }, 1_000)
      return
    }
    releaseAutoScrollGuard()
  }, [releaseAutoScrollGuard, setFollowState, threshold])

  const scheduleScrollToLatest = useCallback((): void => {
    if (!shouldFollowRef.current || scrollFrameRef.current !== null) return
    scrollFrameRef.current = requestAnimationFrame(() => {
      scrollFrameRef.current = null
      if (shouldFollowRef.current) scrollLatest("auto")
    })
  }, [scrollLatest])

  const pause = useCallback((): void => setFollowState(false), [setFollowState])
  const resume = useCallback((): void => setFollowState(true), [setFollowState])
  const scrollToLatest = useCallback((): void => {
    setFollowState(true)
    const reducedMotion = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false
    scrollLatest(reducedMotion ? "auto" : "smooth")
  }, [scrollLatest, setFollowState])

  useEffect(() => {
    const viewport = viewportRef.current
    const content = contentRef.current
    if (viewport === null || content === null) return undefined

    let active = true
    clearProgrammaticGuard()
    cancelScrollFrame()
    setFollowState(true)

    const syncFollowFromScroll = (): void => {
      if (programmaticScrollRef.current) return
      setFollowState(isNearBottom(viewport, threshold))
    }
    const finishProgrammaticScroll = (): void => {
      if (!programmaticScrollRef.current) return
      clearProgrammaticGuard()
      setFollowState(isNearBottom(viewport, threshold))
    }
    const observer = new ResizeObserver(() => {
      if (active && shouldFollowRef.current) scheduleScrollToLatest()
    })

    viewport.addEventListener("scroll", syncFollowFromScroll, { passive: true })
    viewport.addEventListener("scrollend", finishProgrammaticScroll)
    observer.observe(content)
    scheduleScrollToLatest()

    return () => {
      active = false
      observer.disconnect()
      viewport.removeEventListener("scroll", syncFollowFromScroll)
      viewport.removeEventListener("scrollend", finishProgrammaticScroll)
      cancelScrollFrame()
      clearProgrammaticGuard()
    }
  }, [cancelScrollFrame, clearProgrammaticGuard, resetKey, scheduleScrollToLatest, setFollowState, threshold])

  return { viewportRef, contentRef, following, pause, resume, scrollToLatest }
}
