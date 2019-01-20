use std::fmt;
use std::result::Result;

pub fn unwrap_failed<E: fmt::Debug>(msg: &str, error: E) -> ! {
    panic!("{}: {:?}", msg, error);
}

/// Represents a result of an operation which can be completed partially.
///
/// Convertable between `Result<T, E>` and itself for convenience's sake.
#[derive(Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug, Hash)]
#[must_use="this `PartialResult` may contain a `Partial` or `Failure` variant, which should be handled"]
pub enum PartialResult<T, E> {
    Complete(T),
    Partial(T, E),
    Failure(E)
}

use self::PartialResult::*;

impl<T, E> PartialResult<T, E> {
    /// Discard all errors and return only the result, if there was any.
    pub fn ok(self) -> Option<T> {
        match self {
            Complete(result) => Some(result),
            Partial(result, error) => Some(result),
            Failure(error) => None
        }
    }

    /// Discard all results and return only the error, if there was any.
    pub fn err(self) -> Option<E> {
        match self {
            Complete(result) => None,
            Partial(result, error) => Some(error),
            Failure(error) => Some(error)
        }
    }

    /// Discard partial results and return a result only if there was no error.
    pub fn complete(self) -> Result<T, E> {
        match self {
            Complete(result) => Ok(result),
            Partial(result, error) => Err(error),
            Failure(error) => Err(error)
        }
    }

    /// Discard partial errors and return a result only if there was no error.
    pub fn partial(self) -> Result<T, E> {
        match self {
            Complete(result) => Ok(result),
            Partial(result, error) => Ok(result),
            Failure(error) => Err(error)
        }
    }

    /// Obtain the result, if there was any, and the error, if there was any.
    pub fn both(self) -> (Option<T>, Option<E>) {
        match self {
            Complete(result) => (Some(result), None),
            Partial(result, error) => (Some(result), Some(error)),
            Failure(error) => (None, Some(error))
        }
    }
}

impl<T, E> PartialResult<T, E> where E: fmt::Debug {
    /// Obtain the result, panicing if there is none.
    pub fn unwrap(self) -> T {
        match self {
            Complete(result) => result,
            Partial(result, error) => result,
            Failure(error) => unwrap_failed("called `PartialResult::unwrap()` on a `Failure` value", error),
        }
    }

    /// Obtain the result, panicing with a given message if there is none.
    pub fn expect(self, msg: &str) -> T {
        match self {
            Complete(result) => result,
            Partial(result, error) => result,
            Failure(error) => unwrap_failed(msg, error),
        }
    }
}

impl<T, E> PartialResult<T, E> where T: fmt::Debug {
    /// Obtain the error, panicing if there is none.
    pub fn unwrap_err(self) -> E {
        match self {
            Complete(result) => unwrap_failed("called `PartialResult::unwrap_fail()` on a `Complete` value", result),
            Partial(result, error) => error,
            Failure(error) => error,
        }
    }

    /// Obtain the error, panicing with a given message if there is none.
    pub fn expect_err(self, msg: &str) -> E {
        match self {
            Complete(result) => unwrap_failed(msg, result),
            Partial(result, error) => error,
            Failure(error) => error,
        }
    }
}

impl<T, E> From<Result<T, E>> for PartialResult<T, E> {
    fn from(t: Result<T, E>) -> PartialResult<T, E> {
        match t {
            Ok(result) => Complete(result),
            Err(error) => Failure(error)
        }
    }
}

impl<T, E> From<Option<T>> for PartialResult<T, E> where E : Default {
    fn from(t: Option<T>) -> PartialResult<T, E> {
        match t {
            Some(result) => Complete(result),
            None => Failure(E::default())
        }
    }
}
