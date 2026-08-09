import { StrictMode } from "react"
import { createRoot } from "react-dom/client"

import { App } from "@/app"
import { AppErrorBoundary } from "@/components/layout/app-error-boundary"

import "@/index.css"

const root = document.getElementById("root")

if (root === null) {
  throw new Error("GraphLoom Studio root element is missing")
}

createRoot(root).render(
  <StrictMode>
    <AppErrorBoundary>
      <App />
    </AppErrorBoundary>
  </StrictMode>,
)
