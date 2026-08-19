import { cleanup, render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import { listCommunities, listEntities, listRelationships } from "@/api/client"
import { CommunityList, EntityList, RelationshipList } from "@/components/graph/graph-lists"

vi.mock("@/api/client", () => ({
  listCommunities: vi.fn(),
  listEntities: vi.fn(),
  listRelationships: vi.fn(),
}))

const emptyPage = { items: [], next_cursor: null }

beforeEach(() => {
  HTMLElement.prototype.hasPointerCapture = () => false
  HTMLElement.prototype.setPointerCapture = () => undefined
  HTMLElement.prototype.releasePointerCapture = () => undefined
  HTMLElement.prototype.scrollIntoView = () => undefined
  vi.mocked(listCommunities).mockReset().mockResolvedValue(emptyPage)
  vi.mocked(listEntities).mockReset().mockResolvedValue(emptyPage)
  vi.mocked(listRelationships).mockReset().mockResolvedValue(emptyPage)
})
afterEach(cleanup)

describe("Graph list filter submission", () => {
  it("keeps Entity drafts local and uses the applied filter for pagination", async () => {
    vi.mocked(listEntities)
      .mockResolvedValueOnce(emptyPage)
      .mockResolvedValueOnce({
        items: [{ id: "entity-1", short_id: null, title: "Alice", entity_type: "PERSON", degree: null, rank: null, community_ids: [] }],
        next_cursor: "entity-1",
      })
      .mockResolvedValueOnce(emptyPage)
    const user = userEvent.setup()
    render(<EntityList onSelect={vi.fn()} />)
    await waitFor(() => expect(listEntities).toHaveBeenCalledTimes(1))
    expect(vi.mocked(listEntities).mock.calls[0]?.[0]).toMatchObject({ sort: "degree", order: "desc", limit: 50 })

    await user.type(screen.getByLabelText("Entity type filter"), "PERSON")
    expect(listEntities).toHaveBeenCalledTimes(1)
    await user.click(screen.getByLabelText("Apply entity filters"))
    await waitFor(() => expect(listEntities).toHaveBeenCalledTimes(2))
    expect(vi.mocked(listEntities).mock.calls[1]?.[0]).toMatchObject({ type: "PERSON", sort: "degree", order: "desc", limit: 50 })

    await user.clear(screen.getByLabelText("Entity type filter"))
    await user.type(screen.getByLabelText("Entity type filter"), "ORG")
    expect(listEntities).toHaveBeenCalledTimes(2)
    await user.click(screen.getByRole("button", { name: "Load more" }))
    await waitFor(() => expect(listEntities).toHaveBeenCalledTimes(3))
    expect(vi.mocked(listEntities).mock.calls[2]?.[0]).toMatchObject({ type: "PERSON", after: "entity-1", sort: "degree", order: "desc" })
  })

  it("aborts the previous Entity request when a new filter is applied", async () => {
    let resolvePerson: ((value: typeof emptyPage) => void) | undefined
    const personPage = new Promise<typeof emptyPage>((resolve) => { resolvePerson = resolve })
    vi.mocked(listEntities)
      .mockResolvedValueOnce(emptyPage)
      .mockReturnValueOnce(personPage)
      .mockResolvedValueOnce({
        items: [{ id: "entity-2", short_id: null, title: "Acme", entity_type: "ORG", degree: null, rank: null, community_ids: [] }],
        next_cursor: null,
      })
    const user = userEvent.setup()
    render(<EntityList onSelect={vi.fn()} />)
    await waitFor(() => expect(listEntities).toHaveBeenCalledTimes(1))

    const input = screen.getByLabelText("Entity type filter")
    await user.type(input, "PERSON")
    await user.click(screen.getByLabelText("Apply entity filters"))
    await waitFor(() => expect(listEntities).toHaveBeenCalledTimes(2))
    const personSignal = vi.mocked(listEntities).mock.calls[1]?.[1]

    await user.clear(input)
    await user.type(input, "ORG")
    await user.click(screen.getByLabelText("Apply entity filters"))
    await waitFor(() => expect(listEntities).toHaveBeenCalledTimes(3))
    expect(personSignal?.aborted).toBe(true)
    expect(await screen.findByText("Acme")).toBeInTheDocument()
    resolvePerson?.(emptyPage)
    await Promise.resolve()
    expect(screen.getByText("Acme")).toBeInTheDocument()
  })

  it("submits Relationship drafts once when Enter is pressed", async () => {
    const user = userEvent.setup()
    render(<RelationshipList onSelect={vi.fn()} />)
    await waitFor(() => expect(listRelationships).toHaveBeenCalledTimes(1))
    expect(vi.mocked(listRelationships).mock.calls[0]?.[0]).toMatchObject({ sort: "weight", order: "desc", limit: 50 })

    await user.type(screen.getByLabelText("Relationship source filter"), "Alice")
    await user.type(screen.getByLabelText("Relationship target filter"), "Bob")
    expect(listRelationships).toHaveBeenCalledTimes(1)
    await user.keyboard("{Enter}")

    await waitFor(() => expect(listRelationships).toHaveBeenCalledTimes(2))
    expect(vi.mocked(listRelationships).mock.calls[1]?.[0]).toMatchObject({ source: "Alice", target: "Bob", sort: "weight", order: "desc", limit: 50 })
  })

  it("aborts and resets Entity pagination when sort changes while preserving applied filters", async () => {
    let resolveInitial: ((value: typeof emptyPage) => void) | undefined
    const initialPage = new Promise<typeof emptyPage>((resolve) => { resolveInitial = resolve })
    vi.mocked(listEntities)
      .mockReturnValueOnce(initialPage)
      .mockResolvedValueOnce({
        items: [{ id: "entity-2", short_id: null, title: "Alpha", entity_type: "ORG", degree: 2, rank: 1, community_ids: [] }],
        next_cursor: "entity-2",
      })
      .mockResolvedValueOnce({
        items: [{ id: "entity-2", short_id: null, title: "Alpha", entity_type: "ORG", degree: 2, rank: 1, community_ids: [] }],
        next_cursor: "entity-2",
      })
      .mockResolvedValueOnce(emptyPage)
    const user = userEvent.setup()
    render(<EntityList onSelect={vi.fn()} />)
    await waitFor(() => expect(listEntities).toHaveBeenCalledTimes(1))
    const initialSignal = vi.mocked(listEntities).mock.calls[0]?.[1]

    await user.click(screen.getByRole("combobox", { name: "Entity sort" }))
    await user.click(screen.getByRole("option", { name: "Title A–Z" }))

    await waitFor(() => expect(listEntities).toHaveBeenCalledTimes(2))
    expect(initialSignal?.aborted).toBe(true)
    expect(vi.mocked(listEntities).mock.calls[1]?.[0]).toMatchObject({ sort: "title", order: "asc", limit: 50 })
    expect(await screen.findByText("Alpha")).toBeInTheDocument()
    await user.type(screen.getByLabelText("Entity type filter"), "ORG")
    expect(listEntities).toHaveBeenCalledTimes(2)
    await user.click(screen.getByLabelText("Apply entity filters"))
    await waitFor(() => expect(listEntities).toHaveBeenCalledTimes(3))
    expect(vi.mocked(listEntities).mock.calls[2]?.[0]).toMatchObject({ type: "ORG", sort: "title", order: "asc" })
    await user.click(screen.getByRole("button", { name: "Load more" }))
    await waitFor(() => expect(listEntities).toHaveBeenCalledTimes(4))
    expect(vi.mocked(listEntities).mock.calls[3]?.[0]).toMatchObject({ type: "ORG", after: "entity-2", sort: "title", order: "asc" })
    resolveInitial?.(emptyPage)
  })

  it("resets Relationship results and requests page one when sort changes", async () => {
    vi.mocked(listRelationships)
      .mockResolvedValueOnce({ items: [{ id: "relationship-old", short_id: null, source: "Z", target: "A", weight: 1, rank: 2 }], next_cursor: "relationship-old" })
      .mockResolvedValueOnce(emptyPage)
    const user = userEvent.setup()
    render(<RelationshipList onSelect={vi.fn()} />)
    await screen.findByText("Z → A")

    await user.click(screen.getByRole("combobox", { name: "Relationship sort" }))
    await user.click(screen.getByRole("option", { name: "Source A–Z" }))

    await waitFor(() => expect(listRelationships).toHaveBeenCalledTimes(2))
    expect(vi.mocked(listRelationships).mock.calls[1]?.[0]).toMatchObject({ sort: "source", order: "asc", limit: 50 })
    expect(vi.mocked(listRelationships).mock.calls[1]?.[0]).not.toHaveProperty("after")
    expect(screen.queryByText("Z → A")).not.toBeInTheDocument()
  })

  it("parses and submits Community drafts only on Apply", async () => {
    const user = userEvent.setup()
    render(<CommunityList onSelect={vi.fn()} />)
    await waitFor(() => expect(listCommunities).toHaveBeenCalledTimes(1))

    await user.type(screen.getByLabelText("Community level filter"), "1")
    await user.type(screen.getByLabelText("Community parent filter"), "3")
    expect(listCommunities).toHaveBeenCalledTimes(1)
    await user.click(screen.getByLabelText("Apply community filters"))

    await waitFor(() => expect(listCommunities).toHaveBeenCalledTimes(2))
    expect(vi.mocked(listCommunities).mock.calls[1]?.[0]).toMatchObject({ level: 1, parent: 3, limit: 50 })
  })
})
