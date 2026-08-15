pub use walkable_derive::Walkable;

pub trait Visitor: Sized {
    fn visit_item(&mut self, item: &crate::Item) {
        item.walk(self);
    }
    fn visit_assoc_item(&mut self, item: &crate::AssocItem) {
        item.walk(self);
    }
    fn visit_stmt(&mut self, stmt: &crate::Stmt) {
        stmt.walk(self);
    }
    fn visit_expr(&mut self, expr: &crate::Expr) {
        expr.walk(self);
    }
    fn visit_ty(&mut self, ty: &crate::Ty) {
        ty.walk(self);
    }
    fn visit_pat(&mut self, pat: &crate::Pat) {
        pat.walk(self);
    }
    fn visit_block(&mut self, block: &crate::Block) {
        block.walk(self);
    }
    fn visit_ident(&mut self, _ident: &crate::Ident) {}
}

pub trait Walkable<V: Visitor> {
    fn walk(&self, visitor: &mut V);
}

pub trait Visitable<V: Visitor> {
    fn visit(&self, visitor: &mut V);
}

impl<V: Visitor, T: Visitable<V>> Visitable<V> for Vec<T> {
    fn visit(&self, visitor: &mut V) {
        for item in self {
            item.visit(visitor);
        }
    }
}

impl<V: Visitor, T: Visitable<V>> Visitable<V> for Option<T> {
    fn visit(&self, visitor: &mut V) {
        if let Some(item) = self {
            item.visit(visitor);
        }
    }
}

impl<V: Visitor, T: Visitable<V>> Visitable<V> for Box<T> {
    fn visit(&self, visitor: &mut V) {
        (**self).visit(visitor);
    }
}

impl<V: Visitor> Visitable<V> for crate::Span {
    fn visit(&self, _visitor: &mut V) {}
}

impl<V: Visitor> Visitable<V> for String {
    fn visit(&self, _visitor: &mut V) {}
}

impl<V: Visitor> Visitable<V> for bool {
    fn visit(&self, _visitor: &mut V) {}
}

impl<V: Visitor> Visitable<V> for char {
    fn visit(&self, _visitor: &mut V) {}
}

impl<V: Visitor> Visitable<V> for u128 {
    fn visit(&self, _visitor: &mut V) {}
}

impl<V: Visitor> Visitable<V> for crate::Ident {
    fn visit(&self, visitor: &mut V) {
        visitor.visit_ident(self);
    }
}

impl<V: Visitor> Visitable<V> for crate::Expr {
    fn visit(&self, visitor: &mut V) {
        visitor.visit_expr(self);
    }
}

impl<V: Visitor> Visitable<V> for crate::Ty {
    fn visit(&self, visitor: &mut V) {
        visitor.visit_ty(self);
    }
}

impl<V: Visitor> Visitable<V> for crate::Pat {
    fn visit(&self, visitor: &mut V) {
        visitor.visit_pat(self);
    }
}

impl<V: Visitor> Visitable<V> for crate::Stmt {
    fn visit(&self, visitor: &mut V) {
        visitor.visit_stmt(self);
    }
}

impl<V: Visitor> Visitable<V> for crate::Block {
    fn visit(&self, visitor: &mut V) {
        visitor.visit_block(self);
    }
}

impl<V: Visitor, K: Visitable<V>> Walkable<V> for crate::Item<K> {
    fn walk(&self, visitor: &mut V) {
        self.vis.visit(visitor);
        self.annotations.visit(visitor);
        self.kind.visit(visitor);
    }
}

impl<V: Visitor> Visitable<V> for crate::Item<crate::ItemKind> {
    fn visit(&self, visitor: &mut V) {
        visitor.visit_item(self);
    }
}

impl<V: Visitor> Visitable<V> for crate::Item<crate::AssocItemKind> {
    fn visit(&self, visitor: &mut V) {
        visitor.visit_assoc_item(self);
    }
}
