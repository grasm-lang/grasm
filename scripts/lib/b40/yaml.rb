# Oracle for scripts/b40_diff.py: YAML read by Psych (libyaml 0.2.5).
# `ev` prints each file's parse events in the yaml-test-suite's notation,
# `load` its documents as values, both hex-encoded as b40test prints them.
require 'psych'
require 'date'

class Events < Psych::Handler
  attr_reader :out
  def initialize; @out = []; end
  def esc(s); s.gsub("\\") { "\\\\" }.gsub("\n", "\\n").gsub("\t", "\\t").gsub("\r", "\\r").gsub("\b", "\\b"); end
  def props(a, t); (a ? " &#{a}" : "") + (t ? " <#{t}>" : ""); end
  def start_stream(e); @out << "+STR"; end
  def end_stream; @out << "-STR"; end
  def start_document(v, t, implicit); @out << (implicit ? "+DOC" : "+DOC ---"); end
  def end_document(implicit); @out << (implicit ? "-DOC" : "-DOC ..."); end
  def alias(a); @out << "=ALI *#{a}"; end
  def scalar(v, a, t, plain, quoted, style)
    st = { 1 => ":", 2 => "'", 3 => "\"", 4 => "|", 5 => ">" }[style] || ":"
    @out << "=VAL#{props(a, t)} #{st}#{esc(v)}"
  end
  def start_sequence(a, t, i, style); @out << "+SEQ#{style == 2 ? ' []' : ''}#{props(a, t)}"; end
  def end_sequence; @out << "-SEQ"; end
  def start_mapping(a, t, i, style); @out << "+MAP#{style == 2 ? ' {}' : ''}#{props(a, t)}"; end
  def end_mapping; @out << "-MAP"; end
end

def hx(s); s.b.unpack1('H*'); end

def canon(v, seen)
  case v
  when nil then '~'
  when true then 'T'
  when false then 'F'
  when Integer then "i#{v}"
  when Float then v.nan? ? 'fnan' : "f#{[v].pack('d').unpack1('q')}"
  when Symbol then 's' + hx(':' + v.to_s)
  when Date, Time, DateTime then 't'
  when String then v.encoding == Encoding::ASCII_8BIT ? 'x' + hx(v) : 's' + hx(v)
  when Array, Hash
    return "^#{seen[v.object_id]}" if seen.key?(v.object_id)
    me = seen.size
    seen[v.object_id] = me
    if v.is_a?(Array)
      "[#{me}:" + v.map { |x| canon(x, seen) }.join(',') + ']'
    else
      "{#{me}:" + v.map { |k, x| canon(k, {}) + '=' + canon(x, seen) }.sort.join(',') + '}'
    end
  else 'o'
  end
end

mode = ARGV.shift
ARGV.each do |f|
  text = File.read(f, encoding: 'utf-8')
  begin
    if mode == 'ev'
      h = Events.new
      Psych::Parser.new(h).parse(text)
      puts hx(h.out.join("\n"))
    else
      docs = Psych.parse_stream(text).children.map(&:to_ruby)
      puts hx(docs.map { |d| canon(d, {}) }.join("\n"))
    end
  rescue Exception
    puts 'ERR'
  end
end
