use comemo::Prehashed;
use ecow::{EcoString, EcoVec};
use crate::syntax::Span;
use crate::eval::Value;
use crate::model::{Content, Guard, Location, Style, Styles};

/// Attributes that can be attached to content.
#[derive(Debug, Clone, PartialEq, Hash)]
enum Attr {
    Header(ContentHeader),
    Span(Span),
    Value(Prehashed<Value>),
    Child(Prehashed<Content>),
    Styles(Styles),
    Prepared,
    Guard(Guard),
    Location(Location),
    Field(EcoString),
}

#[derive(Debug, Clone)]
struct ContentHeader {
    span_index: Option<usize>,
    styles_index: Option<usize>,
    location_index: Option<usize>,
    prepared: bool,
}

/// Organizes the Content's attributes
/// using a vector.
#[derive(Debug, Clone)]
struct ContentAttrs {
    attrs: EcoVec<Attr>
}

impl ContentAttrs {

    pub(super) fn new() -> Self {
        Self { attrs: EcoVec::new() }
    }

    fn init_header(&mut self) {
        // Append header only when there are no elements.
        if self.attrs.is_empty() {
            self.attrs.push(Attr::Header(ContentHeader {
                span_index: None,
                styles_index: None,
                location_index: None,
                prepared: false,
            }));
        }
    }

    fn get_header(&mut self) {
        self.attrs.
    }

    fn push_attr(&mut self, attr: Attr) {
        self.init_header();
        self.attrs.push(attr);
    }

    pub(super) fn span(&self) -> Option<Span> {
        self.attrs.first()
            .map(Attr::header)
            .flatten()
            .map(|header| header.span_index)
            .flatten()
            .map(|attr| Attr::span(&attr)).flatten()
    }

    pub(super) fn styles(&self) -> Option<&Styles> {
        self.attrs.get(Self::STYLES_INDEX as usize)
            .map(Option::as_ref)
            .flatten()
            .map(|attr| attr.styles())
    }

    pub(super) fn location(&self) -> Option<Location> {
        self.attrs.get(Self::LOCATION_INDEX as usize)
            .map(Option::as_ref)
            .flatten()
            .map(|attr| {
                let Some(Attr::Location(loc)) = attr else {
                    panic!("Could not get content's location");
                };
                loc
            })
    }

    pub(super) fn is_prepared(&self) -> bool {
        self.attrs.get(Self::PREPARED_INDEX as usize).is_some()
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = &Attr> {
        let mut iter = self.attrs.iter();

        /// Consume the iterator enough,
        /// in order to ignore non-field/non-child attributes.
        iter.nth(Self::LOCATION_INDEX as usize);

        iter.map(|attr| attr.as_ref().unwrap())
    }

    /// Attach a field to the content.
    pub(super) fn push_field(&mut self, name: impl Into<EcoString>, value: impl Into<Value>) {
        let name = name.into();
        if let Some(i) = self.attrs.iter().position(|attr| match attr {
            Some(Attr::Field(field)) => *field == name,
            _ => false,
        }) {
            self.attrs.make_mut()[i + 1] = Some(Attr::Value(Prehashed::new(value.into())));
        } else {
            self.attrs.push(Some(Attr::Field(name)));
            self.attrs.push(Some(Attr::Value(Prehashed::new(value.into()))));
        }
    }

    pub(super) fn fields_ref(&self) -> impl Iterator<Item = (&EcoString, &Value)> {
        let mut iter = self.iter();

        std::iter::from_fn(move || {
            let field = iter
                .find_map(|attr| Attr::field(&attr))?;
            let value = iter.next()?.value()?;
            Some((field, value))
        })
    }

    pub(super) fn is_empty(&self) -> bool {
        return self.attrs.len() <= Self::LOCATION_INDEX as usize && self.attrs.iter().all(Option::is_none);
    }

    pub(super) fn is_styled(&self) -> bool {
        self.attrs.get(Self::STYLES_INDEX).is_some()
    }

    /// Style this content with a style entry.
    pub(super) fn styled(mut self, style: impl Into<Style>) -> Self {
        let current_styles: Option<&mut Attr> = self.attrs.make_mut().get_mut(Self::STYLES_INDEX);

        if let Some(Some(prev)) = current_styles.map(Attr::styles_mut) {
            prev.apply_one(style.into());
            self
        } else {
            self.styled_with_map(style.into().into())
        }
    }

    /// Style this content with a full style map.
    pub(super) fn styled_with_map(mut self, styles: Styles) -> Self {
        if styles.is_empty() {
            return self;
        }

        let current_styles: Option<&mut Attr> = self.attrs.make_mut().get_mut(Self::STYLES_INDEX);

        if let Some(Some(prev)) = current_styles.map(Attr::styles_mut) {
            prev.apply(styles);
        } else {
            *current_styles = Some(styles);
        }

        self
    }

    /// Traverse this content
    pub(super) fn traverse<'a, F>(&'a self, this_content: &'a Content, f: &mut F)
        where
            F: FnMut(&'a Content),
    {
        f(this_content);

        for attr in self.iter() {
            match attr {
                Attr::Child(child) => child.traverse(f),
                Attr::Value(value) => walk_value(value, f),
                _ => {}
            }
        }

        /// Walks a given value to find any content that matches the selector.
        fn walk_value<'a, F>(value: &'a Value, f: &mut F)
            where
                F: FnMut(&'a Content),
        {
            match value {
                Value::Content(content) => content.traverse(f),
                Value::Array(array) => {
                    for value in array {
                        walk_value(value, f);
                    }
                }
                _ => {}
            }
        }
    }
}

impl ContentAttrs {
    pub(super) fn set_span(&mut self, value: Span) {
        *self.attrs.make_mut().get_mut(Self::SPAN_INDEX as usize).unwrap() = Some(Attr::Span(value));
    }

    pub(super) fn set_styles(&mut self, value: Styles) {
        *self.attrs.make_mut().get_mut(Self::STYLES_INDEX as usize).unwrap() = Some(Attr::Styles(value));
    }

    pub(super) fn set_prepared(&mut self) {
        *self.attrs.make_mut().get_mut(Self::PREPARED_INDEX as usize).unwrap() = Some(Attr::Prepared);
    }

    pub(super) fn set_location(&mut self, value: Location) {
        *self.attrs.make_mut().get_mut(Self::LOCATION_INDEX as usize).unwrap() = Some(Attr::Location(value));
    }

    pub(super) fn styles_mut(&mut self) -> Option<&mut Styles> {
        self.attrs.make_mut().get_mut(Self::STYLES_INDEX as usize)
            .map(Option::as_mut)
            .flatten()
            .map(|mut attr| attr.styles_mut())
    }

    pub(super) fn iter_mut(&mut self) -> impl Iterator<Item = &mut Attr> {
        let mut iter = self.attrs.make_mut().iter_mut();

        /// Consume the iterator for the first CHILD_ORDER - 1 elements,
        /// in order to ignore non-field/non-child attributes.
        iter.nth(Self::LOCATION_INDEX as usize);

        iter.map(|attr| attr.as_mut().unwrap())
    }
}

impl Attr {
    fn header(&self) -> Option<&ContentHeader> {
        match self {
            Self::Header(header) => Some(header),
            _ => None,
        }
    }

    fn header_mut(&mut self) -> Option<&mut ContentHeader> {
        match self {
            Self::Header(header) => Some(header),
            _ => None,
        }
    }

    pub(super) fn child(&self) -> Option<&Content> {
        match self {
            Self::Child(child) => Some(child),
            _ => None,
        }
    }

    pub(super) fn styles(&self) -> Option<&Styles> {
        match self {
            Self::Styles(styles) => Some(styles),
            _ => None,
        }
    }

    pub(super) fn styles_mut(&mut self) -> Option<&mut Styles> {
        match self {
            Self::Styles(styles) => Some(styles),
            _ => None,
        }
    }

    pub(super) fn field(&self) -> Option<&EcoString> {
        match self {
            Self::Field(field) => Some(field),
            _ => None,
        }
    }

    pub(super) fn value(&self) -> Option<&Value> {
        match self {
            Self::Value(value) => Some(value),
            _ => None,
        }
    }

    pub(super) fn span(&self) -> Option<Span> {
        match self {
            Self::Span(span) => Some(*span),
            _ => None,
        }
    }
}
