use crate::integration::catalog::IntegrationSpec;

#[derive(Clone)]
pub struct IntegrationRow {
    pub key: String,
    pub label: String,
    pub state: String,
    pub category: String,
    pub description: String,
    pub available: bool,
    pub required: bool,
}

pub fn build_integration_rows<FEnabled, FAvailable>(
    all_on: bool,
    catalog: Vec<IntegrationSpec>,
    mut is_enabled: FEnabled,
    mut availability: FAvailable,
) -> Vec<IntegrationRow>
where
    FEnabled: FnMut(&str) -> bool,
    FAvailable: FnMut(&str) -> bool,
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
        required: false,
    });

    for spec in catalog {
        let available = availability(spec.key);
        let enabled = is_enabled(spec.key);
        let state = if spec.required {
            "[required]".to_string()
        } else if enabled && available {
            "[active]".to_string()
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
            required: spec.required,
        });
    }

    rows
}
