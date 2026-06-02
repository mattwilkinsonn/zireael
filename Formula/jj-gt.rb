class JjGt < Formula
  desc "Bridge jj bookmark stacks and Graphite (gt) PR stacks"
  homepage "https://github.com/mattwilkinsonn/zireael/tree/main/tools/jj-gt"
  version "0.3.4"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/jj-gt-v#{version}-darwin-arm64.tar.gz"
      # SHA256 is bumped by .github/workflows/release.yml when a tag is pushed.
      sha256 "6f2535161aa1269449971e2c5d16ebe193d858197df408f104e0d6cacf12a159"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/jj-gt-v#{version}-linux-x64.tar.gz"
      sha256 "359c9a317c6d644b00f4f2ca7f459226e6d6d09881f6253ae2a51d512c477799"
    end
    on_arm do
      url "https://github.com/mattwilkinsonn/zireael/releases/download/v#{version}/jj-gt-v#{version}-linux-arm64.tar.gz"
      sha256 "a526606ea37bd3195a0bdd0fa71a2de69d030e64b2d2602cef45ffd7e8f01d9f"
    end
  end

  def install
    bin.install "jj-gt"
  end

  test do
    assert_match "jj-gt #{version}", shell_output("#{bin}/jj-gt --version")
  end
end
