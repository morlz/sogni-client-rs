use serde_json::json;

pub fn convert(value: Option<f64>, from: &str, to: &str) -> String {
    let Some(value) = value.filter(|value| value.is_finite()) else {
        return json!({"error": "value must be a finite number"}).to_string();
    };
    let from = normalize(from);
    let to = normalize(to);
    let result = if let (Some(source), Some(target)) = (temperature(&from), temperature(&to)) {
        let celsius = match source {
            Temperature::C => value,
            Temperature::F => (value - 32.0) * 5.0 / 9.0,
            Temperature::K => value - 273.15,
        };
        Some(match target {
            Temperature::C => celsius,
            Temperature::F => celsius * 9.0 / 5.0 + 32.0,
            Temperature::K => celsius + 273.15,
        })
    } else {
        ratio(&from, UnitKind::Distance)
            .zip(ratio(&to, UnitKind::Distance))
            .or_else(|| ratio(&from, UnitKind::Weight).zip(ratio(&to, UnitKind::Weight)))
            .or_else(|| ratio(&from, UnitKind::Speed).zip(ratio(&to, UnitKind::Speed)))
            .map(|(source, target)| value * source / target)
    };
    match result.filter(|value| value.is_finite()) {
        Some(result) => json!({
            "input": format!("{value} {from}"),
            "result": format!("{:.6} {to}", result),
            "value": result,
        })
        .to_string(),
        None => json!({
            "error": format!("cannot convert from {from:?} to {to:?}; use compatible temperature, distance, weight, or speed units")
        })
        .to_string(),
    }
}

#[derive(Clone, Copy)]
enum Temperature {
    C,
    F,
    K,
}

fn temperature(unit: &str) -> Option<Temperature> {
    match unit {
        "c" | "celsius" => Some(Temperature::C),
        "f" | "fahrenheit" => Some(Temperature::F),
        "k" | "kelvin" => Some(Temperature::K),
        _ => None,
    }
}

#[derive(Clone, Copy)]
enum UnitKind {
    Distance,
    Weight,
    Speed,
}

fn ratio(unit: &str, kind: UnitKind) -> Option<f64> {
    match (kind, unit) {
        (UnitKind::Distance, "m" | "meter" | "meters") => Some(1.0),
        (UnitKind::Distance, "km" | "kilometer" | "kilometers") => Some(1000.0),
        (UnitKind::Distance, "mi" | "mile" | "miles") => Some(1609.344),
        (UnitKind::Distance, "ft" | "foot" | "feet") => Some(0.3048),
        (UnitKind::Distance, "in" | "inch" | "inches") => Some(0.0254),
        (UnitKind::Distance, "yd" | "yard" | "yards") => Some(0.9144),
        (UnitKind::Distance, "cm" | "centimeter" | "centimeters") => Some(0.01),
        (UnitKind::Distance, "mm" | "millimeter" | "millimeters") => Some(0.001),
        (UnitKind::Weight, "g" | "gram" | "grams") => Some(1.0),
        (UnitKind::Weight, "kg" | "kilogram" | "kilograms") => Some(1000.0),
        (UnitKind::Weight, "lb" | "lbs" | "pound" | "pounds") => Some(453.592),
        (UnitKind::Weight, "oz" | "ounce" | "ounces") => Some(28.3495),
        (UnitKind::Weight, "mg" | "milligram" | "milligrams") => Some(0.001),
        (UnitKind::Weight, "stone" | "stones" | "st") => Some(6350.29),
        (UnitKind::Weight, "ton" | "tons") => Some(907185.0),
        (UnitKind::Weight, "tonne" | "tonnes") => Some(1_000_000.0),
        (UnitKind::Speed, "mph" | "mi/h") => Some(0.44704),
        (UnitKind::Speed, "kph" | "km/h" | "kmh") => Some(0.277778),
        (UnitKind::Speed, "m/s" | "ms" | "m_s") => Some(1.0),
        (UnitKind::Speed, "knots" | "knot" | "kn") => Some(0.514444),
        (UnitKind::Speed, "ft/s") => Some(0.3048),
        _ => None,
    }
}

fn normalize(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace(['°', ' '], "")
}
