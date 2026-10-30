use super::*;

struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        require(count <= self.0.len(), "truncated joint room evidence")?;
        let (value, rest) = self.0.split_at(count);
        self.0 = rest;
        Ok(value)
    }
    fn blob(&mut self, maximum: usize) -> Result<&'a [u8]> {
        let size = u32::from_be_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| invalid("invalid room length"))?,
        ) as usize;
        require(
            size > 0 && size <= maximum,
            "room evidence field exceeds its bound",
        )?;
        self.take(size)
    }
    fn manifest(&mut self) -> Result<Manifest> {
        let generation = u64::from_be_bytes(
            self.take(8)?
                .try_into()
                .map_err(|_| invalid("invalid room generation"))?,
        );
        let mut text = || -> Result<String> {
            let size = u16::from_be_bytes(
                self.take(2)?
                    .try_into()
                    .map_err(|_| invalid("invalid room software length"))?,
            ) as usize;
            require((1..=128).contains(&size), "invalid room software extent")?;
            Ok(std::str::from_utf8(self.take(size)?)
                .map_err(|_| invalid("invalid room software text"))?
                .to_owned())
        };
        let manifest = Manifest {
            generation,
            df_version: text()?,
            dfhack_version: text()?,
        };
        manifest.validate()?;
        Ok(manifest)
    }
}
fn field(out: &mut Vec<u8>, raw: &[u8]) {
    out.extend_from_slice(&(raw.len() as u32).to_be_bytes());
    out.extend_from_slice(raw);
}
impl Definition {
    pub fn decode(raw: &[u8]) -> Result<Self> {
        require(
            raw.len() <= MAX_DEFINITION_SIZE,
            "room definition exceeds its bound",
        )?;
        let mut reader = Reader(raw);
        require(
            reader.take(8)? == b"DFMRCG01",
            "unsupported room definition",
        )?;
        let original =
            MonitorDefinition::decode(reader.blob(construction::origin::MAX_DEFINITION_SIZE)?)?;
        require(reader.0.is_empty(), "trailing room definition bytes")?;
        let result = Self::new(original)?;
        require(
            result.canonical_bytes() == raw,
            "noncanonical room definition",
        )?;
        Ok(result)
    }
}
impl Sample {
    pub fn canonical_bytes(&self) -> Result<Vec<u8>> {
        for raw in [&self.before_capture, &self.after_capture] {
            require(
                !raw.is_empty() && raw.len() <= MAX_MAP_CAPTURE,
                "room map exceeds its bound",
            )?;
        }
        let furnishings = self.furnishings.canonical_bytes()?;
        let mut raw = b"DFMRCS01".to_vec();
        raw.extend_from_slice(&self.before.canonical_bytes()?);
        field(&mut raw, &self.before_capture);
        field(&mut raw, &furnishings);
        raw.extend_from_slice(&self.after.canonical_bytes()?);
        field(&mut raw, &self.after_capture);
        require(
            raw.len() <= MAX_SAMPLE,
            "joint room sample exceeds its bound",
        )?;
        Ok(raw)
    }
    pub fn decode(raw: &[u8]) -> Result<Self> {
        require(
            raw.len() <= MAX_SAMPLE,
            "joint room sample exceeds its bound",
        )?;
        let mut reader = Reader(raw);
        require(
            reader.take(8)? == b"DFMRCS01",
            "unsupported joint room sample",
        )?;
        let before = reader.manifest()?;
        let before_capture = reader.blob(MAX_MAP_CAPTURE)?.to_vec();
        let furnishings = LinkedSample::decode(reader.blob(construction::MAX_SAMPLE)?)?;
        let after = reader.manifest()?;
        let after_capture = reader.blob(MAX_MAP_CAPTURE)?.to_vec();
        require(reader.0.is_empty(), "trailing joint room sample bytes")?;
        let sample = Self {
            before,
            before_capture,
            furnishings,
            after,
            after_capture,
        };
        require(
            sample.canonical_bytes()? == raw,
            "noncanonical joint room sample",
        )?;
        Ok(sample)
    }
}
