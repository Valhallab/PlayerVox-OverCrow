use std::{fmt, marker::PhantomData};

use serde::{
    Deserialize, Deserializer,
    de::{Error as _, IgnoredAny, SeqAccess, Visitor},
};

/// Deserialize a sequence while retaining and allocating space for at most `MAX` entries.
///
/// One additional element is consumed as [`IgnoredAny`] solely to distinguish an exact-fit
/// sequence from an oversized one. The rest of an oversized sequence is never visited.
pub fn deserialize_capped_vec<'de, D, T, const MAX: usize>(
    deserializer: D,
) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct CappedVecVisitor<T, const MAX: usize>(PhantomData<fn() -> T>);

    impl<'de, T, const MAX: usize> Visitor<'de> for CappedVecVisitor<T, MAX>
    where
        T: Deserialize<'de>,
    {
        type Value = Vec<T>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(formatter, "a sequence with at most {MAX} entries")
        }

        fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut values = Vec::with_capacity(MAX);
            while values.len() < MAX {
                match sequence.next_element()? {
                    Some(value) => values.push(value),
                    None => return Ok(values),
                }
            }
            if sequence.next_element::<IgnoredAny>()?.is_some() {
                return Err(A::Error::custom(format_args!(
                    "too many entries (maximum {MAX})"
                )));
            }
            Ok(values)
        }
    }

    deserializer.deserialize_seq(CappedVecVisitor::<T, MAX>(PhantomData))
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use serde::{
        Deserializer,
        de::{self, DeserializeSeed, SeqAccess, Visitor},
        forward_to_deserialize_any,
    };

    use super::deserialize_capped_vec;

    #[derive(Default)]
    struct ReadCounts {
        next_calls: Cell<usize>,
        typed: Cell<usize>,
        ignored: Cell<usize>,
    }

    struct NoHintDeserializer {
        entries: usize,
        counts: Rc<ReadCounts>,
    }

    impl<'de> Deserializer<'de> for NoHintDeserializer {
        type Error = de::value::Error;

        fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
        where
            V: Visitor<'de>,
        {
            self.deserialize_seq(visitor)
        }

        fn deserialize_seq<V>(self, visitor: V) -> Result<V::Value, Self::Error>
        where
            V: Visitor<'de>,
        {
            visitor.visit_seq(NoHintSequence {
                remaining: self.entries,
                counts: self.counts,
            })
        }

        forward_to_deserialize_any! {
            bool i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 char str string bytes byte_buf
            option unit unit_struct newtype_struct tuple tuple_struct map struct enum identifier
            ignored_any
        }
    }

    struct NoHintSequence {
        remaining: usize,
        counts: Rc<ReadCounts>,
    }

    impl<'de> SeqAccess<'de> for NoHintSequence {
        type Error = de::value::Error;

        fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>, Self::Error>
        where
            T: DeserializeSeed<'de>,
        {
            self.counts.next_calls.set(self.counts.next_calls.get() + 1);
            if self.remaining == 0 {
                return Ok(None);
            }
            self.remaining -= 1;
            seed.deserialize(CountingElement {
                counts: Rc::clone(&self.counts),
            })
            .map(Some)
        }

        fn size_hint(&self) -> Option<usize> {
            None
        }
    }

    struct CountingElement {
        counts: Rc<ReadCounts>,
    }

    impl CountingElement {
        fn visit_typed<'de, V>(self, visitor: V) -> Result<V::Value, de::value::Error>
        where
            V: Visitor<'de>,
        {
            self.counts.typed.set(self.counts.typed.get() + 1);
            visitor.visit_u8(7)
        }
    }

    impl<'de> Deserializer<'de> for CountingElement {
        type Error = de::value::Error;

        fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
        where
            V: Visitor<'de>,
        {
            self.visit_typed(visitor)
        }

        fn deserialize_u8<V>(self, visitor: V) -> Result<V::Value, Self::Error>
        where
            V: Visitor<'de>,
        {
            self.visit_typed(visitor)
        }

        fn deserialize_ignored_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
        where
            V: Visitor<'de>,
        {
            self.counts.ignored.set(self.counts.ignored.get() + 1);
            visitor.visit_unit()
        }

        forward_to_deserialize_any! {
            bool i8 i16 i32 i64 u16 u32 u64 f32 f64 char str string bytes byte_buf option unit
            unit_struct newtype_struct seq tuple tuple_struct map struct enum identifier
        }
    }

    #[test]
    fn no_hint_sequence_never_overallocates_or_reads_past_the_overflow_probe() {
        const MAX: usize = 3;
        let exact_counts = Rc::new(ReadCounts::default());
        let values = deserialize_capped_vec::<_, u8, MAX>(NoHintDeserializer {
            entries: MAX,
            counts: Rc::clone(&exact_counts),
        })
        .expect("an exact-cap sequence is valid");
        assert_eq!(values, vec![7; MAX]);
        assert!(
            values.capacity() <= MAX,
            "capacity {} exceeds hard cap {MAX}",
            values.capacity()
        );

        let overflow_counts = Rc::new(ReadCounts::default());
        let error = deserialize_capped_vec::<_, u8, MAX>(NoHintDeserializer {
            entries: MAX + 4,
            counts: Rc::clone(&overflow_counts),
        })
        .expect_err("one overflow probe must reject the sequence");
        assert!(error.to_string().contains("too many entries"));
        assert_eq!(overflow_counts.typed.get(), MAX);
        assert_eq!(overflow_counts.ignored.get(), 1);
        assert_eq!(overflow_counts.next_calls.get(), MAX + 1);
    }
}
