# The oracle for rubycsvline: Ruby 2.6's csv 3.0.9, CSV.parse_line and
# CSV.generate_line. Text is hex of UTF-8 ('-' when empty); 'n' is nil.
require 'csv'

def unhex(h)
  (h == '-' ? '' : [h].pack('H*')).force_encoding('UTF-8')
end

def hx(s)
  s.empty? ? '-' : s.b.unpack1('H*')
end

def opt_s(h)
  h == 'n' ? nil : unhex(h)
end

def parse(f)
  o = { col_sep: unhex(f[2]), row_sep: f[3] == 'a' ? :auto : unhex(f[3]), quote_char: opt_s(f[4]) }
  o[:liberal_parsing] = [false, true, { double_quote_outside_quote: true }, { backslash_quote: true },
                         { double_quote_outside_quote: true, backslash_quote: true }][f[5].to_i]
  o[:strip] = f[6] == 'f' ? false : f[6] == 't' ? true : unhex(f[6][1..-1])
  o[:field_size_limit] = f[7] == 'n' ? nil : f[7].to_i
  o[:skip_blanks] = f[8] == '1'
  o[:nil_value] = opt_s(f[9])
  o[:empty_value] = unhex(f[10])
  row = CSV.parse_line(unhex(f[1]), **o)
  return 'N' if row.nil?
  return '[]' if row.empty?
  row.map { |x| x.nil? ? 'n' : hx(x) }.join(',')
end

def generate(f)
  row = f[1] == '~' ? [] : f[1].split('.').map { |x| opt_s(x) }
  o = { col_sep: unhex(f[2]), row_sep: unhex(f[3]), quote_char: unhex(f[4]), force_quotes: f[5] == '1', quote_empty: f[6] == '1',
        write_nil_value: opt_s(f[7]), write_empty_value: unhex(f[8]) }
  hx(CSV.generate_line(row, **o))
end

out = []
STDIN.each_line do |l|
  f = l.chomp.split(' ')
  next if f.empty?
  begin
    out << (f[0] == 'p' ? parse(f) : generate(f))
  rescue CSV::MalformedCSVError, ArgumentError, EncodingError => e
    out << "E:#{e.class.name.split('::').last}:#{hx(e.message)}"
  end
end
STDOUT.write(out.map { |o| o + "\n" }.join)
