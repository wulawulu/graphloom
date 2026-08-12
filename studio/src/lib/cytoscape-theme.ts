export interface CytoscapeTheme {
  background: string
  foreground: string
  border: string
  person: string
  organization: string
  geo: string
  event: string
  defaultNode: string
  seed: string
}

const tokenNames = {
  background: "--cy-background",
  foreground: "--cy-foreground",
  border: "--cy-border",
  person: "--cy-graph-person",
  organization: "--cy-graph-organization",
  geo: "--cy-graph-geo",
  event: "--cy-graph-event",
  defaultNode: "--cy-graph-default",
  seed: "--cy-graph-seed",
}

const supportedColor = /^#[0-9a-f]{6}$/i

export function readCytoscapeTheme(styles: CSSStyleDeclaration): CytoscapeTheme {
  return {
    background: readToken(styles, tokenNames.background),
    foreground: readToken(styles, tokenNames.foreground),
    border: readToken(styles, tokenNames.border),
    person: readToken(styles, tokenNames.person),
    organization: readToken(styles, tokenNames.organization),
    geo: readToken(styles, tokenNames.geo),
    event: readToken(styles, tokenNames.event),
    defaultNode: readToken(styles, tokenNames.defaultNode),
    seed: readToken(styles, tokenNames.seed),
  }
}

function readToken(styles: CSSStyleDeclaration, token: string): string {
  const value = styles.getPropertyValue(token).trim()
  if (!supportedColor.test(value)) {
    throw new Error(`Invalid Cytoscape renderer theme token: ${token}`)
  }
  return value
}
