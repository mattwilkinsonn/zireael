class JjGt < Formula
  desc "Bridge jj bookmark stacks and Graphite (gt) PR stacks"
  homepage "https://github.com/mattwilkinsonn/zireael/tree/main/tools/jj-gt"
  version "0.3.8"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/jj-gt-v#{version}-darwin-arm64.tar.gz"
      # SHA256 is bumped by .github/workflows/release.yml when a tag is pushed.
      sha256 "36a8f904a5ad6da967c508721834b821aace67001784529c01fb40af00fc5a9c"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/jj-gt-v#{version}-linux-x64.tar.gz"
      sha256 "0206162a64dc4393325e547548bd44c920fe9363caece561fa02cf62ee000dd8"
    end
    on_arm do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/jj-gt-v#{version}-linux-arm64.tar.gz"
      sha256 "b71f97516c9a2e913b97fa3fad25726cdb3f56a7fcdf06c0fbf0c237334ef585"
    end
  end

  def install
    bin.install "jj-gt"
  end

  test do
    assert_match "jj-gt #{version}", shell_output("#{bin}/jj-gt --version")
  end
end
