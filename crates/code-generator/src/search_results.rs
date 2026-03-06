//! SearchResults PHP code generator.

/// Generate the PHP source for a SearchResults class.
pub fn generate_search_results(result_fqcn: &str, source_fqcn: &str) -> String {
    let (ns, class_name) = split_fqcn(result_fqcn);

    format!(
        r#"<?php
namespace {ns};

class {class_name} extends \Magento\Framework\Api\SearchResults
{{
    /**
     * Returns array of items
     *
     * @return \{source}[]
     */
    public function getItems()
    {{
        return parent::getItems();
    }}
}}
"#,
        ns = ns,
        class_name = class_name,
        source = source_fqcn
    )
}

/// Return the file path for a SearchResults class.
pub fn search_results_path(result_fqcn: &str) -> String {
    format!("{}.php", result_fqcn.replace('\\', "/"))
}

fn split_fqcn(fqcn: &str) -> (String, String) {
    match fqcn.rfind('\\') {
        Some(pos) => (fqcn[..pos].to_string(), fqcn[pos + 1..].to_string()),
        None => (String::new(), fqcn.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_results_path() {
        assert_eq!(
            search_results_path("Foo\\Bar\\BazSearchResults"),
            "Foo/Bar/BazSearchResults.php"
        );
    }

    #[test]
    fn test_generate_search_results() {
        let out = generate_search_results(
            "Magento\\Catalog\\Model\\ProductRenderSearchResults",
            "Magento\\Catalog\\Model\\ProductRender",
        );
        assert!(out.contains("namespace Magento\\Catalog\\Model;"));
        assert!(out.contains("class ProductRenderSearchResults"));
        assert!(out.contains("\\Magento\\Catalog\\Model\\ProductRender[]"));
        assert!(out.contains("return parent::getItems();"));
    }
}
