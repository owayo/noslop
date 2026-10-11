//! 文書内の節・リスト・表などの境界。文章の抽出とは別に親子関係を保つ。

use pulldown_cmark::{Event, Parser, Tag};
use serde::Serialize;

use crate::diagnostic::Span;
use crate::document::{Document, SourceFormat};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NodeKind {
    Document,
    Paragraph,
    Heading(u8),
    List,
    Item,
    Table,
    Quote,
    Footnote,
    Code,
    Html,
    Separator,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    pub kind: NodeKind,
    pub span: Span,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub block_ids: Vec<usize>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Structure {
    pub nodes: Vec<Node>,
}

impl Structure {
    /// Markdownと同じfront matter・拡張構文を使う。推定したテキストの構造とは混ぜない。
    pub fn parse(doc: &Document) -> Self {
        if doc.format != SourceFormat::Markdown {
            return Self::default();
        }
        let body = crate::markdown::front_matter_len(&doc.source);
        let mut result = Self {
            nodes: vec![Node {
                kind: NodeKind::Document,
                span: Span::new(body, doc.source.len()),
                parent: None,
                children: vec![],
                block_ids: vec![],
            }],
        };
        let mut stack = vec![Some(0)];
        for (event, range) in
            Parser::new_ext(&doc.source[body..], crate::markdown::options()).into_offset_iter()
        {
            match event {
                Event::Start(tag) => {
                    let kind = match tag {
                        Tag::Paragraph => Some(NodeKind::Paragraph),
                        Tag::Heading { level, .. } => Some(NodeKind::Heading(level as u8)),
                        Tag::List(_) => Some(NodeKind::List),
                        Tag::Item => Some(NodeKind::Item),
                        Tag::Table(_) => Some(NodeKind::Table),
                        Tag::BlockQuote(_) => Some(NodeKind::Quote),
                        Tag::FootnoteDefinition(_) => Some(NodeKind::Footnote),
                        Tag::CodeBlock(_) => Some(NodeKind::Code),
                        Tag::HtmlBlock => Some(NodeKind::Html),
                        _ => None,
                    };
                    let node = kind.map(|kind| {
                        let parent = stack.iter().rev().flatten().next().copied().unwrap();
                        let id = result.nodes.len();
                        result.nodes.push(Node {
                            kind,
                            span: Span::new(range.start + body, range.end + body),
                            parent: Some(parent),
                            children: vec![],
                            block_ids: vec![],
                        });
                        result.nodes[parent].children.push(id);
                        id
                    });
                    stack.push(node);
                }
                Event::End(_) => {
                    stack.pop();
                }
                Event::Rule => {
                    let parent = stack.iter().rev().flatten().next().copied().unwrap();
                    let id = result.nodes.len();
                    result.nodes.push(Node {
                        kind: NodeKind::Separator,
                        span: Span::new(range.start + body, range.end + body),
                        parent: Some(parent),
                        children: vec![],
                        block_ids: vec![],
                    });
                    result.nodes[parent].children.push(id);
                }
                _ => {}
            }
        }
        for node in &mut result.nodes {
            let start = doc
                .blocks
                .partition_point(|b| b.span.start < node.span.start);
            node.block_ids = doc.blocks[start..]
                .iter()
                .enumerate()
                .take_while(|(_, b)| b.span.start < node.span.end)
                .filter(|(_, b)| b.span.end <= node.span.end)
                .map(|(i, _)| i + start)
                .collect();
        }
        result
    }

    /// 引用・脚注・コード内の構造を、本文の関係として数えない。
    pub fn in_body(&self, mut id: usize) -> bool {
        loop {
            let node = &self.nodes[id];
            if matches!(
                node.kind,
                NodeKind::Quote | NodeKind::Footnote | NodeKind::Code
            ) {
                return false;
            }
            match node.parent {
                Some(parent) => id = parent,
                None => return true,
            }
        }
    }

    /// 同じ親の直後の構造。別の節や子リストを結びつけない。
    pub fn next_sibling(&self, id: usize) -> Option<usize> {
        let parent = self.nodes[id].parent?;
        let siblings = &self.nodes[parent].children;
        let index = siblings.iter().position(|&other| other == id)?;
        siblings.get(index + 1).copied()
    }

    /// 見出しから次の同じ階層以上の見出しまで。子見出しの範囲も含む。
    pub fn section_span(&self, id: usize) -> Option<Span> {
        let NodeKind::Heading(level) = self.nodes[id].kind else {
            return None;
        };
        let end = self
            .nodes
            .iter()
            .enumerate()
            .skip(id + 1)
            .find(|(other, node)| {
                self.in_body(*other)
                    && matches!(node.kind, NodeKind::Heading(next) if next <= level)
            })
            .map_or(self.nodes[0].span.end, |(_, node)| node.span.start);
        Some(Span::new(self.nodes[id].span.start, end))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_direct_items_loose_lists_nested_lists_and_offsets() {
        let source = "---\ntitle: メモ\n---\n\n# 案内\n\n次の2項目です。\n\n- 一つ目\n\n  補足の段落。\n\n  - 子の項目\n- 二つ目\n";
        let doc = Document::markdown(source);
        let tree = Structure::parse(&doc);
        let lists: Vec<_> = tree
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::List)
            .collect();
        assert_eq!(lists.len(), 2);
        assert_eq!(lists[0].children.len(), 2);
        assert_eq!(lists[1].children.len(), 1);
        assert_eq!(&source[lists[0].span.range()][..2], "- ");
        let heading = tree
            .nodes
            .iter()
            .position(|n| matches!(n.kind, NodeKind::Heading(_)))
            .unwrap();
        assert_eq!(tree.section_span(heading).unwrap().end, source.len());
    }
}
