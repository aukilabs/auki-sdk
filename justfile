diagnostic-app:
    cargo run -p auki-diagnostic-app

diag: diagnostic-app

generate-swift-bindings crate:
    bash scripts/generate-swift-bindings.sh "{{crate}}"

build-swift-xcframework crate:
    bash scripts/build-swift-xcframework.sh "{{crate}}"
