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
        urlTransform={(url) => url.startsWith(CITATION_URL_PREFIX) ? url : defaultUrlTransform(url)}
        components={{
          img: ({ alt }) => <span>[Remote image omitted{alt === undefined ? "" : `: ${alt}`}]</span>,
          a: ({ children: linkChildren, href }) => {
            const citation = href === undefined ? null : citationGroupFromUrl(href)
            return citation !== null && renderCitation !== undefined
              ? renderCitation(citation)
              : <a href={href} target="_blank" rel="noreferrer noopener">{linkChildren}</a>
          },
        }}
      >
        {children}
      </ReactMarkdown>
    </div>
  )
}
