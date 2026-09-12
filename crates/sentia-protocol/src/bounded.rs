use serde::{de::Error as _, Deserialize, Deserializer, Serialize};
use std::error::Error;
use std::fmt::{self, Display};
use std::ops::Deref;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedStringError {
    pub max: usize,
    pub actual: usize,
}

impl Display for BoundedStringError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "string length {} exceeds maximum {}",
            self.actual, self.max
        )
    }
}

impl Error for BoundedStringError {}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct BoundedString<const MAX: usize>(String);

impl<const MAX: usize> BoundedString<MAX> {
    pub fn new(value: impl Into<String>) -> Result<Self, BoundedStringError> {
        let value = value.into();
        let actual = value.len();
        if actual > MAX {
            return Err(BoundedStringError { max: MAX, actual });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl<const MAX: usize> Display for BoundedString<MAX> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl<const MAX: usize> Deref for BoundedString<MAX> {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl<const MAX: usize> TryFrom<String> for BoundedString<MAX> {
    type Error = BoundedStringError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl<const MAX: usize> TryFrom<&str> for BoundedString<MAX> {
    type Error = BoundedStringError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value.to_owned())
    }
}

impl<const MAX: usize> From<BoundedString<MAX>> for String {
    fn from(value: BoundedString<MAX>) -> Self {
        value.into_string()
    }
}

impl<'de, const MAX: usize> Deserialize<'de> for BoundedString<MAX> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        BoundedString::new(raw).map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedVecError {
    pub max: usize,
    pub actual: usize,
}

impl Display for BoundedVecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "vector length {} exceeds maximum {}",
            self.actual, self.max
        )
    }
}

impl Error for BoundedVecError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct BoundedVec<T, const MAX: usize>(Vec<T>);

impl<T, const MAX: usize> BoundedVec<T, MAX> {
    pub fn new(values: Vec<T>) -> Result<Self, BoundedVecError> {
        let actual = values.len();
        if actual > MAX {
            return Err(BoundedVecError { max: MAX, actual });
        }
        Ok(Self(values))
    }

    pub fn as_slice(&self) -> &[T] {
        self.0.as_slice()
    }

    pub fn into_vec(self) -> Vec<T> {
        self.0
    }
}

impl<T, const MAX: usize> Deref for BoundedVec<T, MAX> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl<T, const MAX: usize> TryFrom<Vec<T>> for BoundedVec<T, MAX> {
    type Error = BoundedVecError;

    fn try_from(value: Vec<T>) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl<T, const MAX: usize> From<BoundedVec<T, MAX>> for Vec<T> {
    fn from(value: BoundedVec<T, MAX>) -> Self {
        value.into_vec()
    }
}

impl<'de, T, const MAX: usize> Deserialize<'de> for BoundedVec<T, MAX>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let values = Vec::<T>::deserialize(deserializer)?;
        BoundedVec::new(values).map_err(D::Error::custom)
    }
}
