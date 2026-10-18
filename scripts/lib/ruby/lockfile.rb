# The reference side of scripts/b18_diff.py for Gemfile.lock: Bundler's own
# LockfileParser. Reads a JSON list of hex-encoded lockfiles on stdin and
# writes a JSON list of answers in the harness's record format.
require "json"
require "tmpdir"

# Bundler wants a Gemfile to stand beside; give it an empty one.
dir = Dir.mktmpdir
File.write(File.join(dir, "Gemfile"), "")
ENV["BUNDLE_GEMFILE"] = File.join(dir, "Gemfile")
Dir.chdir(dir)
require "bundler"

RS = "\u001e"
US = "\u001f"

def kind_of(source)
  case source
  when Bundler::Source::Git then ["git", "#{source.uri}@#{source.options['revision']}"]
  when Bundler::Source::Path then ["path", source.path.to_s]
  else ["gem", ""]
  end
end

def answer(text)
  parser = Bundler::LockfileParser.new(text)
  out = []
  parser.specs.each do |s|
    kind, where = kind_of(s.source)
    deps = s.dependencies.map { |d| "#{d.name}=#{d.requirement}" }.join(";")
    out << ["S", s.name, s.version.to_s, s.platform.to_s, kind, where, deps].join(US)
  end
  parser.dependencies.each_value do |d|
    pinned = d.source ? kind_of(d.source)[0] : ""
    out << ["D", d.name, d.requirement.to_s, pinned].join(US)
  end
  parser.platforms.each { |p| out << "P#{US}#{p}" }
  out << "R#{US}#{parser.ruby_version}"
  out << "B#{US}#{parser.bundler_version}"
  gem = parser.sources.find { |s| s.is_a?(Bundler::Source::Rubygems) }
  out << "G#{US}#{gem ? gem.remotes.map(&:to_s).join(',') : ''}"
  out.join(RS)
rescue Bundler::LockfileError, ArgumentError
  "ERR"
end

inputs = JSON.parse($stdin.read)
$stdout.write(JSON.generate(inputs.map { |h| answer([h].pack("H*").force_encoding("UTF-8")) }))
