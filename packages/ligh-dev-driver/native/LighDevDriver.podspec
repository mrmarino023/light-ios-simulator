Pod::Spec.new do |s|
  s.name         = 'LighDevDriver'
  s.version      = '0.1.0'
  s.summary      = 'LIGH in-process UI driver for Expo development builds and native Debug'
  s.homepage     = 'https://github.com/mrmarino023/light-ios-simulator'
  s.license      = { :type => 'MIT' }
  s.authors      = { 'LIGH' => 'https://github.com/mrmarino023' }
  s.source       = { :git => 'https://github.com/mrmarino023/light-ios-simulator.git', :tag => s.version.to_s }
  s.ios.deployment_target = '15.1'
  s.source_files = '*.{h,m}'
  s.public_header_files = 'LighDevDriver.h'
  s.frameworks   = 'UIKit', 'Foundation', 'QuartzCore'
  s.requires_arc = true
  # Release EAS binaries strip unreferenced ObjC. Keep +load.
  s.user_target_xcconfig = { 'OTHER_LDFLAGS' => '$(inherited) -ObjC' }
end
