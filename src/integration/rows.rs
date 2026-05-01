use crate::integration::catalog::IntegrationSpec;

#[derive(Clone)]
pub struct IntegrationRow {
    pub key: String,
    pub label: String,
    pub state: String,
    pub category: String,
    pub description: String,
    pub available: bool,
    pub partially_supported: bool,
    pub required: bool,
}

pub fn build_integration_rows<FEnabled, FSupport>(
    all_on: bool,
    catalog: Vec<IntegrationSpec>,
    mut is_enabled: FEnabled,
    mut support: FSupport,
) -> Vec<IntegrationRow>
where
    FEnabled: FnMut(&str) -> bool,
    FSupport: FnMut(&str) -> (bool, bool),
{
    let mut rows = Vec::new();
    rows.push(IntegrationRow {
        key: "__all_optional__".to_string(),
        label: "__all_optional__".to_string(),
        state: if all_on {
            "[on]".to_string()
        } else {
            "[off]".to_string()
        },
        category: "global".to_string(),
        description: "Toggle all optional integrations on/off".to_string(),
        available: true,
        partially_supported: false,
        required: false,
    });

    for spec in catalog {
        let (available, partially_supported) = support(spec.key);
        let enabled = is_enabled(spec.key);
        let state = if spec.required {
            "[required]".to_string()
        } else if !available && !partially_supported {
            "[missing]".to_string()
        } else if enabled && available {
            "[active]".to_string()
        } else if enabled && partially_supported {
            "[partial]".to_string()
        } else if enabled {
            "[on]".to_string()
        } else {
            "[off]".to_string()
        };

        rows.push(IntegrationRow {
            key: spec.key.to_string(),
            label: spec.key.to_string(),
            state,
            category: spec.category.to_string(),
            description: spec.description.to_string(),
            available,
            partially_supported,
            required: spec.required,
        });
    }

    rows
}
