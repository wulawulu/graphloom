import { Component, type ReactNode } from "react"
import { TriangleAlert } from "lucide-react"

import { Button } from "@/components/ui/button"

interface Props { children: ReactNode }
interface State { failed: boolean }

export class AppErrorBoundary extends Component<Props, State> {
  state: State = { failed: false }

  static getDerivedStateFromError(): State {
    return { failed: true }
  }

  componentDidCatch(): void {
    // Studio intentionally does not upload render failures or business payloads.
  }

  render(): ReactNode {
    if (!this.state.failed) return this.props.children
    return (
      <main className="flex min-h-screen items-center justify-center bg-background p-6">
        <section className="max-w-md space-y-4 rounded-lg border bg-card p-6 text-center">
          <TriangleAlert className="mx-auto size-8 text-warning" aria-hidden="true" />
          <h1 className="text-lg font-semibold">Studio could not render this view</h1>
          <p className="text-sm text-muted-foreground">Reload the application to recover. No query or graph data was sent anywhere.</p>
          <Button onClick={() => { globalThis.location.reload() }}>Reload Studio</Button>
        </section>
      </main>
    )
  }
}
