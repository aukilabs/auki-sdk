Pod::Spec.new do |s|
  s.name           = 'AukiSdkExpo'
  s.version        = '0.1.0'
  s.summary        = 'Expo module wrapping auki-sdk-swift'
  s.description    = 'Authenticated Auki peer surface for Expo iOS (UniFFI AukiSDK).'
  s.author         = 'Auki Labs'
  s.homepage       = 'https://github.com/aukilabs/auki-sdk'
  s.platforms      = { :ios => '17.0' }
  s.source         = { git: '' }
  s.static_framework = true

  s.dependency 'ExpoModulesCore'

  s.pod_target_xcconfig = {
    'DEFINES_MODULE' => 'YES',
    'SWIFT_COMPILATION_MODE' => 'wholemodule',
  }

  s.source_files = '**/*.{h,m,mm,swift}'
  s.vendored_frameworks = 'Frameworks/AukiSDK.xcframework'
  s.frameworks = 'SystemConfiguration', 'CoreFoundation'
  s.libraries = 'iconv'
end
