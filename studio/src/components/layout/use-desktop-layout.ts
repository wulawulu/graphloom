import { useEffect, useState } from "react"

export function useDesktopLayout(): boolean {
  const [desktop, setDesktop] = useState(() => (
    typeof window.matchMedia !== "function" || window.matchMedia("(min-width: 1024px)").matches
  ))
  useEffect(() => {
    if (typeof window.matchMedia !== "function") return undefined
    const query = window.matchMedia("(min-width: 1024px)")
    const update = (event: MediaQueryListEvent): void => setDesktop(event.matches)
    setDesktop(query.matches)
    query.addEventListener("change", update)
    return () => query.removeEventListener("change", update)
  }, [])
  return desktop
}
