import ReactMarkdown, { defaultUrlTransform } from "react-markdown"
import remarkGfm from "remark-gfm"

import { citationGroupFromUrl, CITATION_URL_PREFIX, remarkDataCitations, type CitationGroup } from "@/lib/citations"

interface SafeMarkdownProps {
  children: string
  className?: string
  renderCitation?: (group: CitationGroup) => React.ReactNode
}

export function SafeMarkdown({ children, className = "markdown-answer", renderCitation }: SafeMarkdownProps): React.ReactElement {
  return (
    <div className={className}>
      <ReactMarkdown
        remarkPlugins={renderCitation === undefined ? [remarkGfm] : [remarkGfm, remarkDataCitations]}
        urlTransform={(url) => renderCitation !== undefined && citationGroupFromUrl(url) !== null ? url : defaultUrlTransform(url)}
        components={{
          img: ({ alt }) => <span>[Remote image omitted{alt === undefined ? "" : `: ${alt}`}]</span>,
          a: ({ children: linkChildren, href }) => {
            const citation = href === undefined ? null : citationGroupFromUrl(href)
            if (citation !== null && renderCitation !== undefined) return renderCitation(citation)
            if (href === undefined || href.length === 0 || href.startsWith(CITATION_URL_PREFIX)) return <span>{linkChildren}</span>
            return <a href={href} target="_blank" rel="noreferrer noopener">{linkChildren}</a>
          },
        }}
      >
        {children}
      </ReactMarkdown>
    </div>
  )
}
