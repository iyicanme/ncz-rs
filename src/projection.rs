//! Geographic to projected coordinates for the coordinate systems of NCZ files: transverse Mercator (Turkish 3° zones
//! and UTM) on the datums that `MPROJ` names.

/// A reference ellipsoid, by its semi-major axis and flattening.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ellipsoid {
    pub a: f64,
    pub f: f64,
}

/// GRS 1980, of TUREF (ITRF96).
pub const GRS80: Ellipsoid = Ellipsoid {
    a: 6_378_137.0,
    f: 1.0 / 298.257_222_101,
};
/// WGS 84.
pub const WGS84: Ellipsoid = Ellipsoid {
    a: 6_378_137.0,
    f: 1.0 / 298.257_223_563,
};
/// International 1924 (Hayford), of ED50.
pub const INTERNATIONAL_1924: Ellipsoid = Ellipsoid {
    a: 6_378_388.0,
    f: 1.0 / 297.0,
};

/// The ellipsoid of an `MPROJ` datum code: 0 WGS 84, 1 TUREF, 4 and 254 ED50. `None` for other codes.
pub fn datum_ellipsoid(datum: u8) -> Option<Ellipsoid> {
    match datum {
        0 => Some(WGS84),
        1 => Some(GRS80),
        4 | 254 => Some(INTERNATIONAL_1924),
        _ => None,
    }
}

/// A transverse Mercator projection with a false easting of 500 km and no false northing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransverseMercator {
    pub ellipsoid: Ellipsoid,
    /// Central meridian, in degrees.
    pub central_meridian: f64,
    /// Scale on the central meridian: 1 for the Turkish 3° zones, 0.9996 for UTM.
    pub scale: f64,
}

impl TransverseMercator {
    /// A Turkish 3° zone (TM27 to TM45), which has scale 1 on its central meridian.
    pub fn turkish_zone(ellipsoid: Ellipsoid, central_meridian: f64) -> Self {
        Self {
            ellipsoid,
            central_meridian,
            scale: 1.0,
        }
    }

    /// The projection an `MPROJ` entry names, `None` when it is not a transverse Mercator projection on a known datum:
    /// projection 3 is a Turkish 3° zone and projection 2 a UTM zone.
    pub fn from_mproj(projection: u8, datum: u8, central_meridian: u8) -> Option<Self> {
        let ellipsoid = datum_ellipsoid(datum)?;
        let scale = match projection {
            2 => 0.9996,
            3 => 1.0,
            _ => return None,
        };
        Some(Self {
            ellipsoid,
            central_meridian: f64::from(central_meridian),
            scale,
        })
    }

    /// The `MPROJ` codes (projection, datum, central meridian) of this projection: projection 3 for a Turkish 3° zone,
    /// 2 for UTM; datum 0 WGS 84, 1 TUREF, 4 ED50. `None` when it has no such codes.
    pub fn mproj_codes(&self) -> Option<(u8, u8, u8)> {
        let projection = if self.scale == 1.0 {
            3
        } else if self.scale == 0.9996 {
            2
        } else {
            return None;
        };
        let datum = if self.ellipsoid == WGS84 {
            0
        } else if self.ellipsoid == GRS80 {
            1
        } else if self.ellipsoid == INTERNATIONAL_1924 {
            4
        } else {
            return None;
        };
        let meridian = self.central_meridian;
        (meridian.fract() == 0.0 && (0.0..=180.0).contains(&meridian))
            .then(|| (projection, datum, meridian as u8))
    }

    /// Projects a longitude and latitude in degrees to easting and northing in metres, with Krüger's series to the
    /// sixth order in the third flattening (Karney 2011), accurate to well under a millimetre within a zone.
    pub fn forward(&self, longitude: f64, latitude: f64) -> (f64, f64) {
        let Ellipsoid { a, f } = self.ellipsoid;
        let n = f / (2.0 - f);
        let (n2, n3) = (n * n, n * n * n);
        let (n4, n5, n6) = (n2 * n2, n2 * n3, n3 * n3);
        let rectifying_radius = a / (1.0 + n) * (1.0 + n2 / 4.0 + n4 / 64.0 + n6 / 256.0);
        let alpha = [
            n / 2.0 - 2.0 * n2 / 3.0 + 5.0 * n3 / 16.0 + 41.0 * n4 / 180.0 - 127.0 * n5 / 288.0
                + 7891.0 * n6 / 37800.0,
            13.0 * n2 / 48.0 - 3.0 * n3 / 5.0 + 557.0 * n4 / 1440.0 + 281.0 * n5 / 630.0
                - 1983433.0 * n6 / 1935360.0,
            61.0 * n3 / 240.0 - 103.0 * n4 / 140.0 + 15061.0 * n5 / 26880.0 + 167603.0 * n6 / 181440.0,
            49561.0 * n4 / 161280.0 - 179.0 * n5 / 168.0 + 6601661.0 * n6 / 7257600.0,
            34729.0 * n5 / 80640.0 - 3418889.0 * n6 / 1995840.0,
            212378941.0 * n6 / 319334400.0,
        ];

        let phi = latitude.to_radians();
        let lambda = (longitude - self.central_meridian).to_radians();
        let e = 2.0 * n.sqrt() / (1.0 + n);
        let t = (phi.sin().atanh() - e * (e * phi.sin()).atanh()).sinh();
        let xi_prime = t.atan2(lambda.cos());
        let eta_prime = (lambda.sin() / (1.0 + t * t).sqrt()).atanh();
        let (mut xi, mut eta) = (xi_prime, eta_prime);
        for (j, alpha) in alpha.iter().enumerate() {
            let k = 2.0 * (j + 1) as f64;
            xi += alpha * (k * xi_prime).sin() * (k * eta_prime).cosh();
            eta += alpha * (k * xi_prime).cos() * (k * eta_prime).sinh();
        }
        let scale = self.scale * rectifying_radius;
        (500_000.0 + scale * eta, scale * xi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_like_proj() {
        // Corners of map sheet F30-A-08-D-1-B in gökgöz_itrf96 (TUREF / TM33), as NetCAD 5 exports them; PROJ gives
        // the same to the millimetre.
        let tm33 = TransverseMercator::turkish_zone(GRS80, 33.0);
        let (x, y) = tm33.forward(33.10625, 41.425);
        assert!((x - 508881.696).abs() < 0.001 && (y - 4587778.398).abs() < 0.001, "{x} {y}");
        let (x, y) = tm33.forward(33.1125, 41.41875);
        assert!((x - 509405.051).abs() < 0.001 && (y - 4587084.920).abs() < 0.001, "{x} {y}");
        // On the central meridian the easting is the false easting.
        let (x, _) = tm33.forward(33.0, 40.0);
        assert!((x - 500_000.0).abs() < 1e-6);
        // UTM zone 36 (central meridian 33) scales by 0.9996.
        let utm = TransverseMercator::from_mproj(2, 0, 33).unwrap();
        let (x, y) = utm.forward(33.0, 40.0);
        let (_, y1) = TransverseMercator::turkish_zone(WGS84, 33.0).forward(33.0, 40.0);
        assert!((x - 500_000.0).abs() < 1e-6 && (y - 0.9996 * y1).abs() < 1e-6);
        assert_eq!(TransverseMercator::from_mproj(0, 0, 0), None);
        assert_eq!(TransverseMercator::from_mproj(3, 255, 33), None);
    }
}
