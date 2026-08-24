import ReactMarkdown, { defaultUrlTransform } from "react-markdown"
import remarkGfm from "remark-gfm"

import {
  citationGroupFromUrl,
  CITATION_PROVENANCE_ATTRIBUTE,
  CITATION_PROVENANCE_VALUE,
  CITATION_URL_PREFIX,
  remarkDataCitations,
  type CitationGroup,
} from "@/lib/citations"

interface SafeMarkdownProps {
  children: string
  className?: string
  renderCitation?: (group: CitationGroup) => React.ReactNode
}

export function SafeMarkdown({ children, className = "markdown-answer min-w-0 max-w-full", renderCitation }: SafeMarkdownProps): React.ReactElement {
  return (
    <div className={className}>
      <ReactMarkdown
        remarkPlugins={renderCitation === undefined ? [remarkGfm] : [remarkGfm, remarkDataCitations]}
        urlTransform={(url) => renderCitation !== undefined && citationGroupFromUrl(url) !== null ? url : defaultUrlTransform(url)}
        components={{
          img: ({ alt }) => <span>[Remote image omitted{alt === undefined ? "" : `: ${alt}`}]</span>,
          table: ({ children }) => <div className="markdown-table-scroll max-w-full overflow-x-auto"><table>{children}</table></div>,
          pre: ({ children }) => <pre className="max-w-full overflow-x-auto">{children}</pre>,
          a: ({ children: linkChildren, href, node }) => {
            const citation = href === undefined ? null : citationGroupFromUrl(href)
            if (isGeneratedCitationNode(node) && citation !== null && renderCitation !== undefined) return renderCitation(citation)
            if (href === undefined || href.length === 0 || href.startsWith(CITATION_URL_PREFIX)) return <span>{linkChildren}</span>
            return <a className="markdown-link" href={href} target="_blank" rel="noreferrer noopener">{linkChildren}</a>
          },
        }}
      >
        {children}
      </ReactMarkdown>
    </div>
  )
}

function isGeneratedCitationNode(value: unknown): boolean {
  if (typeof value !== "object" || value === null) return false
  const properties = (value as { properties?: unknown }).properties
  if (typeof properties !== "object" || properties === null) return false
  return (properties as Record<string, unknown>)[CITATION_PROVENANCE_ATTRIBUTE] === CITATION_PROVENANCE_VALUE
}
