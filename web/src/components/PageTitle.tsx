// Client pages can't export Next metadata; render <title> in the tree instead.
export function PageTitle({ title }: { title: string }) {
  return <title>{title}</title>;
}
