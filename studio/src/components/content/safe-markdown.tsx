import ReactMarkdown from "react-markdown"
import remarkGfm from "remark-gfm"

interface SafeMarkdownProps {
  children: string
  className?: string
}

export function SafeMarkdown({ children, className = "markdown-answer" }: SafeMarkdownProps): React.ReactElement {
  return (
    <div className={className}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          img: ({ alt }) => <span>[Remote image omitted{alt === undefined ? "" : `: ${alt}`}]</span>,
          a: ({ children: linkChildren, href }) => <a href={href} target="_blank" rel="noreferrer noopener">{linkChildren}</a>,
        }}
      >
        {children}
      </ReactMarkdown>
    </div>
  )
}
