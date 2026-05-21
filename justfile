diagnostic-app:
    cargo run -p auki-diagnostic-app

generate-swift-bindings crate:
    bash scripts/generate-swift-bindings.sh "{{crate}}"
    bash scripts/build-swift-xcframework.sh "{{crate}}"

generate-python-bindings crate:
    bash scripts/generate-python-bindings.sh "{{crate}}"
